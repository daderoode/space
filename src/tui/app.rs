use crate::core::{
    config::SpaceConfig,
    git::{FileDiff, FileEntry},
    workspace::{self, Workspace},
};
use crate::tui::actions::StatusKind;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffCacheKey {
    pub repo_index: usize,
    pub path: String,
    pub staged: bool,
}

const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);
const SCROLL_STEP: u16 = 5;

/// Convert a ratatui/crossterm 0.29 KeyEvent into a tui_input InputRequest,
/// bypassing the tui_input crossterm backend which links against crossterm 0.28.
pub(crate) fn key_to_input_request(
    key: &ratatui::crossterm::event::KeyEvent,
) -> Option<tui_input::InputRequest> {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use tui_input::InputRequest;
    match (key.code, key.modifiers) {
        (KeyCode::Backspace, KeyModifiers::NONE) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
            Some(InputRequest::DeletePrevChar)
        }
        (KeyCode::Delete, KeyModifiers::NONE) => Some(InputRequest::DeleteNextChar),
        (KeyCode::Left, KeyModifiers::NONE) | (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
            Some(InputRequest::GoToPrevChar)
        }
        (KeyCode::Left, KeyModifiers::CONTROL) | (KeyCode::Char('b'), KeyModifiers::META) => {
            Some(InputRequest::GoToPrevWord)
        }
        (KeyCode::Right, KeyModifiers::NONE) | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
            Some(InputRequest::GoToNextChar)
        }
        (KeyCode::Right, KeyModifiers::CONTROL) | (KeyCode::Char('f'), KeyModifiers::META) => {
            Some(InputRequest::GoToNextWord)
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Some(InputRequest::DeleteLine),
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => Some(InputRequest::DeletePrevWord),
        (KeyCode::Delete, KeyModifiers::CONTROL) => Some(InputRequest::DeleteNextWord),
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => Some(InputRequest::DeleteTillEnd),
        (KeyCode::Char('a'), KeyModifiers::CONTROL) | (KeyCode::Home, KeyModifiers::NONE) => {
            Some(InputRequest::GoToStart)
        }
        (KeyCode::Char('e'), KeyModifiers::CONTROL) | (KeyCode::End, KeyModifiers::NONE) => {
            Some(InputRequest::GoToEnd)
        }
        (KeyCode::Char(c), KeyModifiers::NONE) => Some(InputRequest::InsertChar(c)),
        (KeyCode::Char(c), KeyModifiers::SHIFT) => Some(InputRequest::InsertChar(c)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pane {
    Left,
    Right,
}

#[derive(Debug)]
pub enum Screen {
    Dashboard,
    CreateWorkspace(crate::tui::screens::create::CreateState),
    GoWorkspace(crate::tui::screens::go::GoState),
    AddRepos(crate::tui::screens::add::AddState),
    ConfirmDelete(crate::tui::screens::delete::DeleteState),
    RepoSearch(crate::tui::screens::search::SearchState),
    ConfigEditor(crate::tui::screens::config::ConfigState),
    DiffViewer(crate::tui::screens::diff::DiffViewerState),
    Help,
}

#[derive(Debug)]
pub enum Message {
    Quit,
    FocusNext,
    SelectWorkspaceUp,
    SelectWorkspaceDown,
    SelectRepoUp,
    SelectRepoDown,
    GoToWorkspace,
    StartGo,
    StartCreate,
    StartAdd,
    StartDelete,
    StartSearch,
    StartConfig,
    RefreshRepos,
    ToggleRepoExpand,
    CollapseAllRepos,
    StartHelp,
    StageFile {
        repo_index: usize,
        path: String,
        currently_staged: bool,
    },
    StageBulk {
        repo_index: usize,
        stage: bool, // true = stage all, false = unstage all
    },
    OpenDiffViewer {
        repo_index: usize,
        path: String,
        staged: bool,
    },
    ScrollTableLeft,
    ScrollTableRight,
}

/// A row in the flattened repo table (repo header or file entry).
#[allow(dead_code)] // fields consumed by renderer (Task 5) and tests
pub enum RepoRow<'a> {
    Repo {
        index: usize,
        repo: &'a crate::core::workspace::WorkspaceRepo,
        expanded: bool,
    },
    /// Section divider within an expanded repo: "Conflicts", "Unstaged", or "Staged".
    SectionHeader {
        repo_index: usize,
        label: &'static str,
    },
    /// A file entry within an expanded repo.
    /// `partially_staged` is true when the same path appears in both the
    /// Staged and Unstaged sections (some hunks staged, some not).
    File {
        repo_index: usize,
        entry: &'a FileEntry,
        partially_staged: bool,
    },
}

pub struct App {
    pub config: SpaceConfig,
    pub workspaces: Vec<Workspace>,
    pub repos_cache: Vec<PathBuf>,
    pub selected_ws: usize,
    pub selected_repo: usize,
    pub expanded_repos: HashSet<usize>,
    pub repo_file_cache: HashMap<usize, Vec<FileEntry>>,
    pub diff_content_cache: HashMap<DiffCacheKey, Result<FileDiff, String>>,
    pub file_mtime_cache: HashMap<DiffCacheKey, std::time::SystemTime>,
    pub repo_index_mtime: HashMap<usize, std::time::SystemTime>,
    pub cursor_row: usize,
    pub table_scroll_x: u16,
    pub focus: Pane,
    pub screen: Screen,
    pub should_quit: bool,
    pub space_cd_target: Option<PathBuf>,
    pub status_message: Option<String>,
    pub status_kind: StatusKind,
    pub status_message_set_at: Option<Instant>,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = SpaceConfig::load()?;
        let workspaces = workspace::list_workspaces(&config.workspaces.dir)?;
        // Load repo cache; rescan if missing or stale
        let cache_path = SpaceConfig::cache_path();
        let repos_cache = crate::core::repo::load_cache(&cache_path, config.repos.cache_age_secs)
            .unwrap_or_else(|| {
                let found =
                    crate::core::repo::find_repos_in(&config.repos.roots, config.repos.max_depth);
                crate::core::repo::save_cache(&cache_path, &found).ok();
                found
            });
        let mut app = Self {
            config,
            workspaces,
            repos_cache,
            selected_ws: 0,
            selected_repo: 0,
            expanded_repos: HashSet::new(),
            repo_file_cache: HashMap::new(),
            diff_content_cache: HashMap::new(),
            file_mtime_cache: HashMap::new(),
            repo_index_mtime: HashMap::new(),
            cursor_row: 0,
            table_scroll_x: 0,
            focus: Pane::Left,
            screen: Screen::Dashboard,
            should_quit: false,
            space_cd_target: None,
            status_message: None,
            status_kind: StatusKind::Info,
            status_message_set_at: None,
        };
        app.load_selected_workspace_detail();
        Ok(app)
    }

    pub fn selected_workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.selected_ws)
    }

    /// Build the flat list of rows for the repo table.
    ///
    /// Each expanded repo emits section headers (Conflicts / Unstaged / Staged)
    /// followed by the file entries in each group. Empty groups are omitted.
    ///
    /// # Invariant
    /// Every `SectionHeader` row is always immediately followed by at least one
    /// `File` row (empty sections are never emitted). The first and last rows of
    /// the list are therefore always `Repo` or `File` rows — never `SectionHeader`.
    /// `skip_headers` and `reposition_after_section_change` rely on this.
    pub fn flattened_rows(&self) -> Vec<RepoRow<'_>> {
        let repos = match self.selected_workspace() {
            Some(ws) => &ws.repos,
            None => return vec![],
        };
        let mut rows = Vec::new();
        for (i, repo) in repos.iter().enumerate() {
            let expanded = self.expanded_repos.contains(&i);
            rows.push(RepoRow::Repo {
                index: i,
                repo,
                expanded,
            });
            if expanded {
                if let Some(entries) = self.repo_file_cache.get(&i) {
                    use crate::core::git::FileStatus;
                    let conflicts: Vec<_> = entries
                        .iter()
                        .filter(|e| e.status == FileStatus::Conflicted)
                        .collect();
                    let unstaged: Vec<_> = entries
                        .iter()
                        .filter(|e| !e.staged && e.status != FileStatus::Conflicted)
                        .collect();
                    let staged: Vec<_> = entries
                        .iter()
                        .filter(|e| e.staged && e.status != FileStatus::Conflicted)
                        .collect();

                    // Detect partially-staged files: same path in both staged and unstaged groups
                    let partially_staged_paths: std::collections::HashSet<&str> = {
                        let staged_set: std::collections::HashSet<&str> =
                            staged.iter().map(|e| e.path.as_str()).collect();
                        unstaged
                            .iter()
                            .filter(|e| staged_set.contains(e.path.as_str()))
                            .map(|e| e.path.as_str())
                            .collect()
                    };

                    if !conflicts.is_empty() {
                        rows.push(RepoRow::SectionHeader {
                            repo_index: i,
                            label: "Conflicts",
                        });
                        for entry in &conflicts {
                            rows.push(RepoRow::File {
                                repo_index: i,
                                entry,
                                partially_staged: false, // conflicts can't be partially staged
                            });
                        }
                    }
                    if !unstaged.is_empty() {
                        rows.push(RepoRow::SectionHeader {
                            repo_index: i,
                            label: "Unstaged",
                        });
                        for entry in &unstaged {
                            rows.push(RepoRow::File {
                                repo_index: i,
                                entry,
                                partially_staged: partially_staged_paths
                                    .contains(entry.path.as_str()),
                            });
                        }
                    }
                    if !staged.is_empty() {
                        rows.push(RepoRow::SectionHeader {
                            repo_index: i,
                            label: "Staged",
                        });
                        for entry in &staged {
                            rows.push(RepoRow::File {
                                repo_index: i,
                                entry,
                                partially_staged: partially_staged_paths
                                    .contains(entry.path.as_str()),
                            });
                        }
                    }
                }
            }
        }
        rows
    }

    /// Return the repo index the cursor is on (whether on a Repo, SectionHeader, or File row).
    pub fn repo_index_for_cursor(&self) -> Option<usize> {
        match self.flattened_rows().get(self.cursor_row) {
            Some(RepoRow::Repo { index, .. }) => Some(*index),
            Some(RepoRow::SectionHeader { repo_index, .. }) => Some(*repo_index),
            Some(RepoRow::File { repo_index, .. }) => Some(*repo_index),
            None => None,
        }
    }

    pub fn load_selected_workspace_detail(&mut self) {
        if let Some(ws) = self.workspaces.get(self.selected_ws) {
            let name = ws.name.clone();
            match workspace::workspace_detail(&self.config.workspaces.dir, &name) {
                Ok(detail) => {
                    self.workspaces[self.selected_ws] = detail;
                }
                Err(_e) => {
                    // Keep shallow workspace entry; note error for user
                    self.set_status(
                        format!("Could not load '{}' detail", name),
                        StatusKind::Error,
                    );
                }
            }
        }
        self.refresh_file_diff_cache();
    }

    /// Reset all repo-pane state that is keyed to the current workspace.
    /// Must be called whenever `selected_ws` changes so stale rows/cursor
    /// positions from the previous workspace are never visible.
    pub fn reset_repo_pane_state(&mut self) {
        self.cursor_row = 0;
        self.table_scroll_x = 0;
        self.expanded_repos.clear();
        self.repo_file_cache.clear();
        self.diff_content_cache.clear();
        self.file_mtime_cache.clear();
        self.repo_index_mtime.clear();
    }

    /// Fetch file diffs for all repos in the selected workspace and populate
    /// `repo_file_cache`. Called on workspace load/switch so the `+/-` column
    /// shows file line totals even on collapsed rows.
    pub fn refresh_file_diff_cache(&mut self) {
        self.repo_file_cache.clear();
        self.diff_content_cache.clear();
        self.file_mtime_cache.clear();
        let repo_paths: Vec<(usize, std::path::PathBuf)> = self
            .selected_workspace()
            .map(|ws| {
                ws.repos
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (i, r.path.clone()))
                    .collect()
            })
            .unwrap_or_default();

        let mut failures = 0usize;
        for (idx, path) in repo_paths {
            match crate::core::git::file_diff(&path) {
                Ok(entries) => {
                    self.repo_file_cache.insert(idx, entries);
                    // Record .git/index mtime for staleness detection
                    if let Some(mtime) = crate::core::git::git_index_mtime(&path) {
                        self.repo_index_mtime.insert(idx, mtime);
                    }
                }
                Err(_) => {
                    failures += 1;
                }
            }
        }
        if failures > 0 {
            self.set_status(
                format!("Diff failed for {} repo(s)", failures),
                StatusKind::Error,
            );
        }
    }

    /// Stage or unstage a single file, invalidate caches, and set a status message.
    /// Shared by `Message::StageFile` (dashboard) and `ScreenAction::StageFile` (diff overlay).
    fn do_stage(
        &mut self,
        repo_index: usize,
        repo_path: &std::path::Path,
        path: &str,
        currently_staged: bool,
    ) {
        // Block staging on conflicted files
        if let Some(entries) = self.repo_file_cache.get(&repo_index) {
            if entries
                .iter()
                .any(|e| e.path == path && e.status == crate::core::git::FileStatus::Conflicted)
            {
                self.set_status(
                    "Cannot stage conflicted file \u{2014} resolve conflicts first".to_string(),
                    StatusKind::Warning,
                );
                return;
            }
        }

        let result = if currently_staged {
            crate::core::git::unstage_file(repo_path, path)
        } else {
            crate::core::git::stage_file(repo_path, path)
        };
        match result {
            Ok(()) => {
                // Invalidate diff content cache entries for this repo
                self.diff_content_cache
                    .retain(|key, _| key.repo_index != repo_index);
                self.file_mtime_cache
                    .retain(|key, _| key.repo_index != repo_index);
                // Re-fetch file list for this repo
                match crate::core::git::file_diff(repo_path) {
                    Ok(entries) => {
                        self.repo_file_cache.insert(repo_index, entries);
                    }
                    Err(_) => {
                        // Keep stale cache entry — better than empty UI
                        let verb = if currently_staged {
                            "Unstaged"
                        } else {
                            "Staged"
                        };
                        self.set_status(
                            format!("{} {} -- refresh failed, press r", verb, path),
                            StatusKind::Warning,
                        );
                        return;
                    }
                }
                let verb = if currently_staged {
                    "Unstaged"
                } else {
                    "Staged"
                };
                self.set_status(format!("{} {}", verb, path), StatusKind::Success);
            }
            Err(err) => {
                let verb = if currently_staged { "Unstage" } else { "Stage" };
                self.set_status(format!("{} failed: {}", verb, err), StatusKind::Error);
            }
        }
    }

    fn set_status(&mut self, msg: impl Into<String>, kind: StatusKind) {
        self.status_message = Some(msg.into());
        self.status_kind = kind;
        self.status_message_set_at = Some(Instant::now());
    }

    fn clear_status(&mut self) {
        self.status_message = None;
        self.status_message_set_at = None;
    }

    fn expire_status_message(&mut self, now: Instant) {
        if let Some(set_at) = self.status_message_set_at {
            if now.duration_since(set_at) >= STATUS_MESSAGE_TTL {
                self.clear_status();
            }
        }
    }

    /// Refresh workspace list when leaving the Creating stage.
    /// Catches partially-created worktrees from error scenarios.
    fn refresh_if_leaving_creating_stage(&mut self) {
        let is_creating = matches!(
            &self.screen,
            Screen::CreateWorkspace(st) if st.stage == crate::tui::screens::create::CreateStage::Creating
        ) || matches!(
            &self.screen,
            Screen::AddRepos(st) if st.stage == crate::tui::screens::add::AddStage::Creating
        );
        if is_creating {
            if let Ok(ws) = crate::core::workspace::list_workspaces(&self.config.workspaces.dir) {
                self.workspaces = ws;
                self.selected_ws = 0;
                self.reset_repo_pane_state();
                self.load_selected_workspace_detail();
            }
        }
    }

    fn execute_worktree_flow(&mut self, params: crate::tui::actions::WorktreeParams) {
        use crate::core::workspace::create_worktree;

        // Clear progress/error on whichever screen is active
        match &mut self.screen {
            Screen::CreateWorkspace(st) => {
                st.progress.clear();
                st.error = None;
            }
            Screen::AddRepos(st) => {
                st.progress.clear();
                st.error = None;
            }
            _ => return,
        }

        let verb = if params.is_new { "Creating" } else { "Adding" };

        for repo_path in &params.repos {
            let repo_name = repo_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".to_string());

            // Push progress message
            match &mut self.screen {
                Screen::CreateWorkspace(st) => {
                    st.progress
                        .push(format!("{} worktree for {}...", verb, repo_name));
                }
                Screen::AddRepos(st) => {
                    st.progress
                        .push(format!("{} worktree for {}...", verb, repo_name));
                }
                _ => return,
            }

            match create_worktree(
                repo_path,
                &params.workspace_dir,
                &params.workspace_name,
                &params.branch_strategy,
            ) {
                Ok(_) => match &mut self.screen {
                    Screen::CreateWorkspace(st) => {
                        st.progress.push(format!("  \u{2713} {}", repo_name));
                    }
                    Screen::AddRepos(st) => {
                        st.progress.push(format!("  \u{2713} {}", repo_name));
                    }
                    _ => return,
                },
                Err(e) => {
                    if e.to_string().contains("already checked out") {
                        match &mut self.screen {
                            Screen::CreateWorkspace(st) => {
                                st.stage =
                                    crate::tui::screens::create::CreateStage::PickBranchStrategy;
                                st.progress.clear();
                                st.error = Some(format!(
                                    "'{}' is already checked out — pick a different strategy",
                                    repo_name
                                ));
                            }
                            Screen::AddRepos(st) => {
                                st.stage = crate::tui::screens::add::AddStage::PickBranchStrategy;
                                st.progress.clear();
                                st.error = Some(format!(
                                    "'{}' is already checked out — pick a different strategy",
                                    repo_name
                                ));
                            }
                            _ => {}
                        }
                        return;
                    }
                    match &mut self.screen {
                        Screen::CreateWorkspace(st) => {
                            st.progress.push(format!("  \u{2717} {}: {}", repo_name, e));
                            st.error = Some(format!("Failed: {}", e));
                        }
                        Screen::AddRepos(st) => {
                            st.progress.push(format!("  \u{2717} {}: {}", repo_name, e));
                            st.error = Some(format!("Failed: {}", e));
                        }
                        _ => return,
                    }
                }
            }
        }

        // Check result — need fresh borrow after the loop
        let had_error = match &self.screen {
            Screen::CreateWorkspace(st) => st.error.is_some(),
            Screen::AddRepos(st) => st.error.is_some(),
            _ => false,
        };

        if !had_error {
            if let Ok(ws_list) = crate::core::workspace::list_workspaces(&params.workspace_dir) {
                self.workspaces = ws_list;
                if let Some(idx) = self
                    .workspaces
                    .iter()
                    .position(|w| w.name == params.workspace_name)
                {
                    self.selected_ws = idx;
                }
                self.reset_repo_pane_state();
                self.load_selected_workspace_detail();
            }
            let verb = if params.is_new {
                "Created"
            } else {
                "Added repos to"
            };
            self.screen = Screen::Dashboard;
            self.set_status(
                format!("{} workspace '{}'", verb, params.workspace_name),
                StatusKind::Success,
            );
        }
        // If error, stay on Creating stage so user can see the log
    }

    fn process_action(&mut self, action: crate::tui::actions::ScreenAction) {
        use crate::tui::actions::ScreenAction;
        match action {
            ScreenAction::Continue => {}
            ScreenAction::Back => {
                // Refresh workspaces when leaving Creating stage (catches partial creates)
                self.refresh_if_leaving_creating_stage();
                self.screen = Screen::Dashboard;
            }
            ScreenAction::BackWithStatus(msg, kind) => {
                self.refresh_if_leaving_creating_stage();
                self.screen = Screen::Dashboard;
                self.set_status(msg, kind);
            }
            ScreenAction::CdAndQuit(path) => {
                self.space_cd_target = Some(path);
                self.should_quit = true;
            }
            ScreenAction::DeleteWorkspace { name, force } => {
                let ws_dir = self.config.workspaces.dir.clone();
                match crate::core::workspace::remove_workspace(&ws_dir, &name, force) {
                    Ok(()) => {
                        if let Ok(ws) = crate::core::workspace::list_workspaces(&ws_dir) {
                            self.workspaces = ws;
                            self.selected_ws = 0;
                        }
                        self.reset_repo_pane_state();
                        self.load_selected_workspace_detail();
                        self.screen = Screen::Dashboard;
                        self.set_status(
                            format!("Deleted workspace '{}'", name),
                            StatusKind::Success,
                        );
                    }
                    Err(e) => {
                        self.screen = Screen::Dashboard;
                        self.set_status(format!("Delete failed: {}", e), StatusKind::Error);
                    }
                }
            }
            ScreenAction::ExecuteWorktreeFlow(params) => {
                self.execute_worktree_flow(params);
            }
            ScreenAction::SaveConfig(new_config) => {
                self.config = new_config;
                self.screen = Screen::Dashboard;
                self.set_status("Config saved", StatusKind::Success);
            }
            ScreenAction::NavigateToWorkspace(repo_name) => {
                self.screen = Screen::Dashboard;
                let found_idx = self
                    .workspaces
                    .iter()
                    .position(|ws| ws.repos.iter().any(|r| r.name == repo_name));
                if let Some(idx) = found_idx {
                    self.selected_ws = idx;
                    self.selected_repo = 0;
                    self.reset_repo_pane_state();
                    self.load_selected_workspace_detail();
                } else {
                    self.set_status(
                        "Not in any workspace — use 'c' to create one",
                        StatusKind::Info,
                    );
                }
            }
            ScreenAction::StageFile {
                repo_index,
                repo_path,
                path,
                currently_staged,
            } => {
                self.do_stage(repo_index, &repo_path, &path, currently_staged);
                self.screen = Screen::Dashboard;
            }
        }
    }

    /// Process a single key press, dispatching to the appropriate screen handler
    /// or mapping to a Message for the Dashboard.
    ///
    /// Extracted from `run_loop()` so tests can drive state transitions without
    /// a terminal or event loop.
    #[allow(clippy::drop_non_drop)] // drop(ctx) releases shared borrows before &mut self
    pub fn handle_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};

        // Global: Ctrl-C always quits (raw mode swallows the OS signal)
        if key.code == KeyCode::Char('c')
            && key
                .modifiers
                .contains(ratatui::crossterm::event::KeyModifiers::CONTROL)
        {
            self.should_quit = true;
            return;
        }

        // Build read-only context from split borrows (disjoint from &mut self.screen)
        let ctx = crate::tui::actions::ScreenContext {
            config: &self.config,
        };

        let action = match &mut self.screen {
            Screen::Help => crate::tui::screens::help::handle_key(key, &ctx),
            Screen::ConfirmDelete(state) => state.handle_key(key, &ctx),
            Screen::GoWorkspace(state) => state.handle_key(key, &ctx),
            Screen::RepoSearch(state) => state.handle_key(key, &ctx),
            Screen::CreateWorkspace(state) => state.handle_key(key, &ctx),
            Screen::AddRepos(state) => state.handle_key(key, &ctx),
            Screen::ConfigEditor(state) => state.handle_key(key, &ctx),
            Screen::DiffViewer(state) => state.handle_key(key, &ctx),
            Screen::Dashboard => {
                drop(ctx);
                // Dashboard key-to-message mapping
                let msg: Option<Message> = match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _) => Some(Message::Quit),
                    (KeyCode::Tab, _) => Some(Message::FocusNext),
                    // Enter: context-sensitive
                    (KeyCode::Enter, _) => match self.focus {
                        Pane::Left => Some(Message::GoToWorkspace),
                        Pane::Right => {
                            let rows = self.flattened_rows();
                            match rows.get(self.cursor_row) {
                                Some(RepoRow::Repo { .. }) => Some(Message::ToggleRepoExpand),
                                Some(RepoRow::File {
                                    repo_index, entry, ..
                                }) => Some(Message::OpenDiffViewer {
                                    repo_index: *repo_index,
                                    path: entry.path.clone(),
                                    staged: entry.staged,
                                }),
                                Some(RepoRow::SectionHeader { .. }) | None => None,
                            }
                        }
                    },
                    (KeyCode::Char('g'), _) => Some(Message::StartGo),
                    (KeyCode::Char('c'), _) => Some(Message::StartCreate),
                    (KeyCode::Char('a'), _) => Some(Message::StartAdd),
                    (KeyCode::Char('d'), _) => Some(Message::StartDelete),
                    (KeyCode::Char('r'), _) => Some(Message::RefreshRepos),
                    (KeyCode::Char('/'), _) => Some(Message::StartSearch),
                    (KeyCode::Char('?'), _) => Some(Message::StartHelp),
                    // s/space: stage/unstage single file (Right pane only)
                    (KeyCode::Char('s') | KeyCode::Char(' '), _) if self.focus == Pane::Right => {
                        let rows = self.flattened_rows();
                        if let Some(RepoRow::File {
                            repo_index, entry, ..
                        }) = rows.get(self.cursor_row)
                        {
                            Some(Message::StageFile {
                                repo_index: *repo_index,
                                path: entry.path.clone(),
                                currently_staged: entry.staged,
                            })
                        } else {
                            None
                        }
                    }
                    // S: stage all (Right pane) or open config (Left pane)
                    (KeyCode::Char('S'), _) => match self.focus {
                        Pane::Right => {
                            self.repo_index_for_cursor()
                                .map(|repo_index| Message::StageBulk {
                                    repo_index,
                                    stage: true,
                                })
                        }
                        Pane::Left => Some(Message::StartConfig),
                    },
                    // U: unstage all (Right pane only)
                    (KeyCode::Char('U'), _) if self.focus == Pane::Right => self
                        .repo_index_for_cursor()
                        .map(|repo_index| Message::StageBulk {
                            repo_index,
                            stage: false,
                        }),
                    (KeyCode::Char('h'), KeyModifiers::NONE) if self.focus == Pane::Right => {
                        Some(Message::ScrollTableLeft)
                    }
                    (KeyCode::Char('l'), KeyModifiers::NONE) if self.focus == Pane::Right => {
                        Some(Message::ScrollTableRight)
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => match self.focus {
                        Pane::Left => Some(Message::SelectWorkspaceUp),
                        Pane::Right => Some(Message::SelectRepoUp),
                    },
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => match self.focus {
                        Pane::Left => Some(Message::SelectWorkspaceDown),
                        Pane::Right => Some(Message::SelectRepoDown),
                    },
                    // Right arrow: context-sensitive
                    (KeyCode::Right, _) => match self.focus {
                        Pane::Left => Some(Message::FocusNext),
                        Pane::Right => {
                            let rows = self.flattened_rows();
                            match rows.get(self.cursor_row) {
                                Some(RepoRow::Repo { .. }) => Some(Message::ToggleRepoExpand),
                                _ => None,
                            }
                        }
                    },
                    // Left arrow and Esc: collapse-first on right pane
                    (KeyCode::Left, _) => match self.focus {
                        Pane::Left => None,
                        Pane::Right => {
                            if self.expanded_repos.is_empty() {
                                Some(Message::FocusNext)
                            } else {
                                Some(Message::CollapseAllRepos)
                            }
                        }
                    },
                    (KeyCode::Esc, _) => match self.focus {
                        Pane::Left => Some(Message::Quit),
                        Pane::Right => {
                            if self.expanded_repos.is_empty() {
                                Some(Message::FocusNext)
                            } else {
                                Some(Message::CollapseAllRepos)
                            }
                        }
                    },
                    _ => None,
                };
                if let Some(m) = msg {
                    let mut next = update(self, m);
                    while let Some(m2) = next {
                        next = update(self, m2);
                    }
                }
                return;
            }
        };

        self.process_action(action);
    }
}

/// Advance `cursor_row` in the given direction, skipping `SectionHeader` rows.
/// Returns the new cursor position.
///
/// # Invariant
/// `flattened_rows()` guarantees every `SectionHeader` is immediately followed
/// (or preceded) by at least one `Repo` or `File` row — empty sections are never
/// emitted. The boundary escape paths therefore always land on a non-header row.
/// The `debug_assert` below catches any future violation in debug builds.
fn skip_headers(rows: &[RepoRow<'_>], from: usize, down: bool) -> usize {
    let max = rows.len().saturating_sub(1);
    let mut pos = from;
    loop {
        if down {
            if pos >= max {
                break;
            }
            pos += 1;
        } else {
            if pos == 0 {
                break;
            }
            pos -= 1;
        }
        if !matches!(rows[pos], RepoRow::SectionHeader { .. }) {
            return pos;
        }
    }
    debug_assert!(
        !matches!(rows.get(pos), Some(RepoRow::SectionHeader { .. })),
        "skip_headers boundary landed on SectionHeader at {pos} — flattened_rows invariant violated"
    );
    pos
}

/// After a staging operation, cursor may rest on a SectionHeader.
/// Try advancing forward to the next non-header row; if none, retreat backward.
///
/// See the invariant note on `skip_headers` — the `flattened_rows()` guarantee
/// ensures this always resolves to a non-header. The `debug_assert` catches
/// violations in debug builds.
fn reposition_after_section_change(rows: &[RepoRow<'_>], cursor: usize) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let max = rows.len().saturating_sub(1);
    let cursor = cursor.min(max);
    // Try advancing forward past any header
    let mut pos = cursor;
    while pos < max && matches!(rows[pos], RepoRow::SectionHeader { .. }) {
        pos += 1;
    }
    // If still on a header (e.g. last row is a header), retreat
    while pos > 0 && matches!(rows[pos], RepoRow::SectionHeader { .. }) {
        pos -= 1;
    }
    debug_assert!(
        !matches!(rows.get(pos), Some(RepoRow::SectionHeader { .. })),
        "reposition_after_section_change landed on SectionHeader at {pos} — invariant violated"
    );
    pos
}

pub fn update(app: &mut App, msg: Message) -> Option<Message> {
    match msg {
        Message::Quit => {
            app.should_quit = true;
            None
        }
        Message::FocusNext => {
            app.focus = match app.focus {
                Pane::Left => Pane::Right,
                Pane::Right => Pane::Left,
            };
            None
        }
        Message::SelectWorkspaceUp => {
            if app.selected_ws > 0 {
                app.selected_ws -= 1;
                app.selected_repo = 0;
                app.reset_repo_pane_state();
                app.load_selected_workspace_detail();
            }
            None
        }
        Message::SelectWorkspaceDown => {
            if app.selected_ws + 1 < app.workspaces.len() {
                app.selected_ws += 1;
                app.selected_repo = 0;
                app.reset_repo_pane_state();
                app.load_selected_workspace_detail();
            }
            None
        }
        Message::SelectRepoUp => {
            let rows = app.flattened_rows();
            app.cursor_row = skip_headers(&rows, app.cursor_row, false);
            None
        }
        Message::SelectRepoDown => {
            let rows = app.flattened_rows();
            app.cursor_row = skip_headers(&rows, app.cursor_row, true);
            None
        }
        Message::RefreshRepos => {
            let roots = app.config.repos.roots.clone();
            let depth = app.config.repos.max_depth;
            let repos = crate::core::repo::find_repos_in(&roots, depth);
            let _ = crate::core::repo::save_cache(&SpaceConfig::cache_path(), &repos);
            app.repos_cache = repos;
            app.cursor_row = 0;
            app.expanded_repos.clear();
            app.refresh_file_diff_cache();
            app.set_status(
                format!("Refreshed: {} repos found", app.repos_cache.len()),
                StatusKind::Success,
            );
            None
        }
        Message::GoToWorkspace => {
            if let Some(ws) = app.selected_workspace() {
                app.space_cd_target = Some(ws.path.clone());
                app.should_quit = true;
            }
            None
        }
        Message::StartCreate => {
            let state =
                crate::tui::screens::create::CreateState::new(app.repos_cache.clone(), vec![]);
            app.screen = Screen::CreateWorkspace(state);
            None
        }
        Message::StartGo => {
            let state = crate::tui::screens::go::GoState::new(&app.workspaces);
            app.screen = Screen::GoWorkspace(state);
            None
        }
        Message::StartAdd => {
            if let Some(ws) = app.selected_workspace() {
                let existing: std::collections::HashSet<_> =
                    ws.repos.iter().map(|r| r.name.clone()).collect();
                let available: Vec<_> = app
                    .repos_cache
                    .iter()
                    .filter(|p| {
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        !existing.contains(&name)
                    })
                    .cloned()
                    .collect();
                let state =
                    crate::tui::screens::add::AddState::new(ws.name.clone(), available, vec![]);
                app.screen = Screen::AddRepos(state);
            }
            None
        }
        Message::StartDelete => {
            if let Some(ws) = app.selected_workspace() {
                let state = crate::tui::screens::delete::DeleteState {
                    workspace_name: ws.name.clone(),
                    repo_names: ws.repos.iter().map(|r| r.name.clone()).collect(),
                };
                app.screen = Screen::ConfirmDelete(state);
            }
            None
        }
        Message::StartSearch => {
            let state = crate::tui::screens::search::SearchState::new(app.repos_cache.clone());
            app.screen = Screen::RepoSearch(state);
            None
        }
        Message::StartConfig => {
            let state = crate::tui::screens::config::ConfigState::from_config(&app.config);
            app.screen = Screen::ConfigEditor(state);
            None
        }
        Message::ToggleRepoExpand => {
            if let Some(idx) = app.repo_index_for_cursor() {
                if app.expanded_repos.contains(&idx) {
                    // Collapsing: remove from expanded, snap cursor to the repo row
                    app.expanded_repos.remove(&idx);
                    let snap_pos = app
                        .flattened_rows()
                        .iter()
                        .position(|r| matches!(r, crate::tui::app::RepoRow::Repo { index, .. } if *index == idx))
                        .unwrap_or(0);
                    app.cursor_row = snap_pos;
                } else {
                    // Expanding: fetch diffs and cache
                    if let Some(repo_path) = app
                        .selected_workspace()
                        .and_then(|ws| ws.repos.get(idx))
                        .map(|r| r.path.clone())
                    {
                        match crate::core::git::file_diff(&repo_path) {
                            Ok(entries) => {
                                app.repo_file_cache.insert(idx, entries);
                                if let Some(mtime) = crate::core::git::git_index_mtime(&repo_path) {
                                    app.repo_index_mtime.insert(idx, mtime);
                                }
                            }
                            Err(err) => {
                                app.set_status(format!("Diff failed: {}", err), StatusKind::Error);
                                return None;
                            }
                        }
                    }
                    app.expanded_repos.insert(idx);
                }
            }
            None
        }
        Message::StartHelp => {
            app.screen = Screen::Help;
            None
        }
        Message::CollapseAllRepos => {
            let current_repo_idx = app.repo_index_for_cursor().unwrap_or(0);
            app.expanded_repos.clear();
            // Snap cursor to the repo row it was in
            let repos_len = app
                .selected_workspace()
                .map(|ws| ws.repos.len())
                .unwrap_or(0);
            app.cursor_row = current_repo_idx.min(repos_len.saturating_sub(1));
            None
        }
        Message::StageFile {
            repo_index,
            path,
            currently_staged,
        } => {
            let repo_path = app
                .selected_workspace()
                .and_then(|ws| ws.repos.get(repo_index))
                .map(|r| r.path.clone());
            let repo_path = match repo_path {
                Some(p) => p,
                None => {
                    app.set_status("Stage failed: repo not found", StatusKind::Error);
                    return None;
                }
            };
            app.do_stage(repo_index, &repo_path, &path, currently_staged);
            // Adjust cursor: clamp and skip headers after the file moved sections
            let rows = app.flattened_rows();
            app.cursor_row = reposition_after_section_change(&rows, app.cursor_row);
            None
        }
        Message::StageBulk { repo_index, stage } => {
            let repo_path = app
                .selected_workspace()
                .and_then(|ws| ws.repos.get(repo_index))
                .map(|r| r.path.clone());
            let repo_path = match repo_path {
                Some(p) => p,
                None => {
                    app.set_status("Stage failed: repo not found", StatusKind::Error);
                    return None;
                }
            };
            let result = if stage {
                crate::core::git::stage_all_unstaged(&repo_path)
            } else {
                crate::core::git::unstage_all_staged(&repo_path)
            };
            match result {
                Ok(count) => {
                    // Invalidate diff content cache entries for this repo
                    app.diff_content_cache
                        .retain(|key, _| key.repo_index != repo_index);
                    app.file_mtime_cache
                        .retain(|key, _| key.repo_index != repo_index);
                    // Re-fetch file list for this repo
                    match crate::core::git::file_diff(&repo_path) {
                        Ok(entries) => {
                            app.repo_file_cache.insert(repo_index, entries);
                        }
                        Err(_) => {
                            // Keep stale cache entry — better than empty UI
                            let verb = if stage { "Staged" } else { "Unstaged" };
                            app.set_status(
                                format!("{} {} file(s) -- refresh failed, press r", verb, count),
                                StatusKind::Warning,
                            );
                            return None;
                        }
                    }
                    let verb = if stage { "Staged" } else { "Unstaged" };
                    app.set_status(format!("{} {} file(s)", verb, count), StatusKind::Success);
                }
                Err(err) => {
                    let verb = if stage { "Stage" } else { "Unstage" };
                    app.set_status(format!("{} failed: {}", verb, err), StatusKind::Error);
                    // Operation failed — section structure unchanged, no reposition needed.
                    return None;
                }
            }
            // Reposition cursor after section structure changed
            let rows = app.flattened_rows();
            app.cursor_row = reposition_after_section_change(&rows, app.cursor_row);
            None
        }
        Message::OpenDiffViewer {
            repo_index,
            path,
            staged,
        } => {
            let (repo_path, repo_name) = match app
                .selected_workspace()
                .and_then(|ws| ws.repos.get(repo_index))
            {
                Some(r) => (r.path.clone(), r.name.clone()),
                None => {
                    app.set_status("Diff viewer: repo not found", StatusKind::Error);
                    return None;
                }
            };

            // Check if .git/index has changed since cache was populated
            let current_mtime = crate::core::git::git_index_mtime(&repo_path);
            let cached_mtime = app.repo_index_mtime.get(&repo_index);
            let stale = match (current_mtime, cached_mtime) {
                (Some(current), Some(cached)) => &current != cached,
                (Some(_), None) => true, // mtime exists but wasn't recorded -- treat as stale
                _ => false,              // can't read mtime -- skip check
            };
            if stale {
                // Invalidate diff content cache for this repo
                app.diff_content_cache
                    .retain(|key, _| key.repo_index != repo_index);
                app.file_mtime_cache
                    .retain(|key, _| key.repo_index != repo_index);
                // Update recorded mtime
                if let Some(mtime) = current_mtime {
                    app.repo_index_mtime.insert(repo_index, mtime);
                }
            }

            let cache_key = DiffCacheKey {
                repo_index,
                path: path.clone(),
                staged,
            };

            // Check if the specific working-tree file has changed
            if !staged {
                // Only relevant for unstaged diffs — staged diffs depend on the index, already covered
                let file_full_path = repo_path.join(&path);
                let current_file_mtime = std::fs::metadata(&file_full_path)
                    .and_then(|m| m.modified())
                    .ok();
                let cached_file_mtime = app.file_mtime_cache.get(&cache_key);
                let file_stale = match (current_file_mtime, cached_file_mtime) {
                    (Some(current), Some(cached)) => &current != cached,
                    (Some(_), None) => false, // first time — not stale, will be recorded after caching
                    _ => false,
                };
                if file_stale {
                    app.diff_content_cache.remove(&cache_key);
                }
            }

            let diff = if let Some(cached) = app.diff_content_cache.get(&cache_key) {
                cached.clone()
            } else {
                let result = crate::core::git::file_content_diff(&repo_path, &path, staged)
                    .map_err(|e| e.to_string());
                app.diff_content_cache
                    .insert(cache_key.clone(), result.clone());
                // Record the file's mtime for future staleness checks
                if !staged {
                    let file_full_path = repo_path.join(&path);
                    if let Ok(meta) = std::fs::metadata(&file_full_path) {
                        if let Ok(mtime) = meta.modified() {
                            app.file_mtime_cache.insert(cache_key, mtime);
                        }
                    }
                }
                result
            };

            let total_lines = diff
                .as_ref()
                .map(|d| u16::try_from(d.lines.len()).unwrap_or(u16::MAX))
                .unwrap_or(0);

            let state = crate::tui::screens::diff::DiffViewerState {
                repo_index,
                repo_name,
                repo_path,
                file_path: path,
                staged,
                diff,
                scroll_offset: 0,
                total_lines,
            };
            app.screen = Screen::DiffViewer(state);
            None
        }
        Message::ScrollTableLeft => {
            app.table_scroll_x = app.table_scroll_x.saturating_sub(SCROLL_STEP);
            None
        }
        Message::ScrollTableRight => {
            app.table_scroll_x = app.table_scroll_x.saturating_add(SCROLL_STEP);
            None
        }
    }
}

/// Entry point — initialise terminal, run event loop, restore terminal.
/// Returns a path to cd into, if the user pressed enter on a workspace.
pub fn run(app: &mut App) -> Result<()> {
    color_eyre::install().ok();
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, app);
    ratatui::restore();
    result
}

/// Build a FuzzyPicker populated with local + remote branches from `repo_path`.
/// Returns `None` if branch listing fails (not a git repo, etc.).
pub(crate) fn build_branch_picker(
    repo_path: &std::path::Path,
    repo_name: &str,
) -> Option<crate::tui::widgets::fuzzy_picker::FuzzyPicker> {
    use crate::core::git::list_branches;
    use crate::tui::widgets::fuzzy_picker::{FuzzyPicker, PickerItem};

    let branches = list_branches(repo_path).ok()?;
    if branches.is_empty() {
        return None;
    }

    let items: Vec<PickerItem> = branches
        .into_iter()
        .map(|b| PickerItem {
            // `name` is what gets passed to git — must be the clean branch name.
            // Indicate the current branch via the `parent` field shown in the picker.
            name: b.name,
            parent: match (b.is_remote, b.is_current) {
                (_, true) => "current".to_string(),
                (true, false) => "remote".to_string(),
                (false, false) => "local".to_string(),
            },
            full_path: std::path::PathBuf::new(), // unused for branch picker
            branch: None,
            remote_url: None,
        })
        .collect();

    Some(FuzzyPicker::new(
        format!("Branch  ({})  ENTER=select  ESC=back", repo_name),
        items,
        false,
    ))
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    use ratatui::crossterm::event::{self, Event, KeyEventKind};

    // Drain any stale input that accumulated before the TUI started —
    // e.g. keystrokes typed during a previous frozen/crashed session that
    // left the terminal in raw mode.  Without this, buffered events replay
    // immediately into the first field, corrupting it.
    while event::poll(std::time::Duration::ZERO)? {
        let _ = event::read()?;
    }

    loop {
        app.expire_status_message(Instant::now());
        terminal.draw(|frame| crate::tui::ui::view(app, frame))?;

        // Post-draw: clamp table_scroll_x to the actual max for the current terminal
        // size, so 'h' always responds immediately rather than unwinding an overshoot.
        if let Ok(size) = terminal.size() {
            let right_pane = ((size.width as f64 * 75.0) / 100.0).round() as usize;
            let inner = right_pane.saturating_sub(2);
            let max = crate::tui::ui::max_table_scroll(inner);
            app.table_scroll_x = app.table_scroll_x.min(max);
        }

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                app.handle_key(key);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::git::RepoStatus;
    use crate::core::workspace::{Workspace, WorkspaceRepo};
    use std::time::{Duration, Instant};

    fn make_app(workspaces: Vec<Workspace>) -> App {
        App {
            config: SpaceConfig::default(),
            workspaces,
            repos_cache: vec![],
            selected_ws: 0,
            selected_repo: 0,
            expanded_repos: HashSet::new(),
            repo_file_cache: HashMap::new(),
            diff_content_cache: HashMap::new(),
            file_mtime_cache: HashMap::new(),
            repo_index_mtime: HashMap::new(),
            cursor_row: 0,
            table_scroll_x: 0,
            focus: Pane::Left,
            screen: Screen::Dashboard,
            should_quit: false,
            space_cd_target: None,
            status_message: None,
            status_kind: StatusKind::Info,
            status_message_set_at: None,
        }
    }

    fn stub_repo(name: &str) -> WorkspaceRepo {
        WorkspaceRepo {
            name: name.to_string(),
            path: std::path::PathBuf::from("/tmp").join(name),
            branch: "main".to_string(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }
    }

    fn app_with_status() -> App {
        let mut app = make_app(vec![]);
        app.set_status(
            "Added repos to workspace 'mission-control-ui'",
            StatusKind::Success,
        );
        app
    }

    #[test]
    fn status_message_expires_after_timeout() {
        let mut app = app_with_status();
        app.status_message_set_at =
            Some(Instant::now() - STATUS_MESSAGE_TTL - Duration::from_millis(1));

        app.expire_status_message(Instant::now());

        assert_eq!(app.status_message, None);
    }

    #[test]
    fn navigate_to_workspace_resets_scroll_and_cursor() {
        let ws0 = Workspace {
            name: "alpha".to_string(),
            path: std::path::PathBuf::from("/tmp/alpha"),
            repos: vec![stub_repo("repo-a")],
        };
        let ws1 = Workspace {
            name: "beta".to_string(),
            path: std::path::PathBuf::from("/tmp/beta"),
            repos: vec![stub_repo("repo-b")],
        };
        let mut app = make_app(vec![ws0, ws1]);

        // Simulate stale state from a previous workspace
        app.table_scroll_x = 20;
        app.cursor_row = 3;

        // Dispatch NavigateToWorkspace targeting a repo in workspace 1
        app.process_action(crate::tui::actions::ScreenAction::NavigateToWorkspace(
            "repo-b".to_string(),
        ));

        assert_eq!(
            app.selected_ws, 1,
            "selected_ws should switch to workspace 1"
        );
        assert_eq!(app.table_scroll_x, 0, "table_scroll_x must be reset to 0");
        assert_eq!(app.cursor_row, 0, "cursor_row must be reset to 0");
    }
}

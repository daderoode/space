use crate::core::{
    config::SpaceConfig,
    git::{DiffTarget, FileEntry},
    workspace::{self, Workspace},
};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);

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
    ToggleDiffTarget, // wired up fully in Task 6, just declare it here
}

/// A row in the flattened repo table (repo header or file entry).
#[allow(dead_code)] // fields consumed by renderer (Task 5) and tests
pub enum RepoRow<'a> {
    Repo {
        index: usize,
        repo: &'a crate::core::workspace::WorkspaceRepo,
        expanded: bool,
    },
    File {
        repo_index: usize,
        entry: &'a FileEntry,
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
    pub cursor_row: usize,
    pub diff_target: DiffTarget,
    pub focus: Pane,
    pub screen: Screen,
    pub should_quit: bool,
    pub space_cd_target: Option<PathBuf>,
    pub status_message: Option<String>,
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
            cursor_row: 0,
            diff_target: DiffTarget::Head,
            focus: Pane::Left,
            screen: Screen::Dashboard,
            should_quit: false,
            space_cd_target: None,
            status_message: None,
            status_message_set_at: None,
        };
        app.load_selected_workspace_detail();
        Ok(app)
    }

    pub fn selected_workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.selected_ws)
    }

    /// Build the flat list of rows for the repo table.
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
                    for entry in entries {
                        rows.push(RepoRow::File {
                            repo_index: i,
                            entry,
                        });
                    }
                }
            }
        }
        rows
    }

    /// Return the repo index the cursor is on (whether on a Repo or File row).
    pub fn repo_index_for_cursor(&self) -> Option<usize> {
        match self.flattened_rows().get(self.cursor_row) {
            Some(RepoRow::Repo { index, .. }) => Some(*index),
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
                    self.set_status(format!("Could not load '{}' detail", name));
                }
            }
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
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
                self.load_selected_workspace_detail();
            }
            let verb = if params.is_new {
                "Created"
            } else {
                "Added repos to"
            };
            self.screen = Screen::Dashboard;
            self.set_status(format!("{} workspace '{}'", verb, params.workspace_name));
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
            ScreenAction::BackWithStatus(msg) => {
                self.refresh_if_leaving_creating_stage();
                self.screen = Screen::Dashboard;
                self.set_status(msg);
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
                        self.load_selected_workspace_detail();
                        self.screen = Screen::Dashboard;
                        self.set_status(format!("Deleted workspace '{}'", name));
                    }
                    Err(e) => {
                        self.screen = Screen::Dashboard;
                        self.set_status(format!("Delete failed: {}", e));
                    }
                }
            }
            ScreenAction::ExecuteWorktreeFlow(params) => {
                self.execute_worktree_flow(params);
            }
            ScreenAction::SaveConfig(new_config) => {
                self.config = new_config;
                self.screen = Screen::Dashboard;
                self.set_status("Config saved");
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
                    self.load_selected_workspace_detail();
                } else {
                    self.set_status("Not in any workspace — use 'c' to create one");
                }
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
        use ratatui::crossterm::event::KeyCode;

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
            Screen::ConfirmDelete(state) => state.handle_key(key, &ctx),
            Screen::GoWorkspace(state) => state.handle_key(key, &ctx),
            Screen::RepoSearch(state) => state.handle_key(key, &ctx),
            Screen::CreateWorkspace(state) => state.handle_key(key, &ctx),
            Screen::AddRepos(state) => state.handle_key(key, &ctx),
            Screen::ConfigEditor(state) => state.handle_key(key, &ctx),
            Screen::Dashboard => {
                drop(ctx);
                // Dashboard key-to-message mapping
                let msg: Option<Message> = match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _) => Some(Message::Quit),
                    (KeyCode::Tab, _) => Some(Message::FocusNext),
                    // Enter: context-sensitive
                    (KeyCode::Enter, _) => match self.focus {
                        Pane::Left => Some(Message::GoToWorkspace),
                        Pane::Right => Some(Message::ToggleRepoExpand),
                    },
                    (KeyCode::Char('g'), _) => Some(Message::StartGo),
                    (KeyCode::Char('c'), _) => Some(Message::StartCreate),
                    (KeyCode::Char('a'), _) => Some(Message::StartAdd),
                    (KeyCode::Char('d'), _) => Some(Message::StartDelete),
                    (KeyCode::Char('r'), _) => Some(Message::RefreshRepos),
                    (KeyCode::Char('T'), _) => Some(Message::ToggleDiffTarget),
                    (KeyCode::Char('/'), _) => Some(Message::StartSearch),
                    (KeyCode::Char('S'), _) => Some(Message::StartConfig),
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
                        Pane::Right => Some(Message::ToggleRepoExpand),
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
                app.cursor_row = 0;
                app.expanded_repos.clear();
                app.repo_file_cache.clear();
                app.load_selected_workspace_detail();
            }
            None
        }
        Message::SelectWorkspaceDown => {
            if app.selected_ws + 1 < app.workspaces.len() {
                app.selected_ws += 1;
                app.selected_repo = 0;
                app.cursor_row = 0;
                app.expanded_repos.clear();
                app.repo_file_cache.clear();
                app.load_selected_workspace_detail();
            }
            None
        }
        Message::SelectRepoUp => {
            if app.cursor_row > 0 {
                app.cursor_row -= 1;
            }
            None
        }
        Message::SelectRepoDown => {
            let max = app.flattened_rows().len().saturating_sub(1);
            if app.cursor_row < max {
                app.cursor_row += 1;
            }
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
            app.repo_file_cache.clear();
            app.set_status(format!("Refreshed: {} repos found", app.repos_cache.len()));
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
                        let entries = crate::core::git::file_diff(&repo_path, &app.diff_target)
                            .unwrap_or_default();
                        app.repo_file_cache.insert(idx, entries);
                    }
                    app.expanded_repos.insert(idx);
                }
            }
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
        Message::ToggleDiffTarget => {
            app.diff_target = match app.diff_target {
                DiffTarget::Head => DiffTarget::Base,
                DiffTarget::Base => DiffTarget::Head,
            };
            // Re-fetch diffs for all currently expanded repos
            let expanded: Vec<usize> = app.expanded_repos.iter().copied().collect();
            for idx in expanded {
                if let Some(repo_path) = app
                    .selected_workspace()
                    .and_then(|ws| ws.repos.get(idx))
                    .map(|r| r.path.clone())
                {
                    let entries = crate::core::git::file_diff(&repo_path, &app.diff_target)
                        .unwrap_or_default();
                    app.repo_file_cache.insert(idx, entries);
                }
            }
            let label = match app.diff_target {
                DiffTarget::Head => "HEAD (uncommitted changes)",
                DiffTarget::Base => "base branch (total divergence)",
            };
            app.set_status(format!("Diff target: {}", label));
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
    use std::time::{Duration, Instant};

    fn app_with_status() -> App {
        let mut app = App {
            config: SpaceConfig::default(),
            workspaces: vec![],
            repos_cache: vec![],
            selected_ws: 0,
            selected_repo: 0,
            expanded_repos: HashSet::new(),
            repo_file_cache: HashMap::new(),
            cursor_row: 0,
            diff_target: DiffTarget::Head,
            focus: Pane::Left,
            screen: Screen::Dashboard,
            should_quit: false,
            space_cd_target: None,
            status_message: None,
            status_message_set_at: None,
        };
        app.set_status("Added repos to workspace 'mission-control-ui'");
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
}

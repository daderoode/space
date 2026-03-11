use crate::core::{
    config::SpaceConfig,
    workspace::{self, Workspace},
};
use anyhow::Result;
use std::path::PathBuf;

/// Convert a ratatui/crossterm 0.29 KeyEvent into a tui_input InputRequest,
/// bypassing the tui_input crossterm backend which links against crossterm 0.28.
fn key_to_input_request(
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
}

pub struct App {
    pub config: SpaceConfig,
    pub workspaces: Vec<Workspace>,
    pub repos_cache: Vec<PathBuf>,
    pub selected_ws: usize,
    pub selected_repo: usize,
    pub focus: Pane,
    pub screen: Screen,
    pub should_quit: bool,
    pub space_cd_target: Option<PathBuf>,
    pub status_message: Option<String>,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = SpaceConfig::load()?;
        let workspaces = workspace::list_workspaces(&config.workspaces.dir)?;
        // Load repo cache if available
        let repos_cache =
            crate::core::repo::load_cache(&SpaceConfig::cache_path()).unwrap_or_default();
        let mut app = Self {
            config,
            workspaces,
            repos_cache,
            selected_ws: 0,
            selected_repo: 0,
            focus: Pane::Left,
            screen: Screen::Dashboard,
            should_quit: false,
            space_cd_target: None,
            status_message: None,
        };
        app.load_selected_workspace_detail();
        Ok(app)
    }

    pub fn selected_workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.selected_ws)
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
                    self.status_message = Some(format!("Could not load '{}' detail", name));
                }
            }
        }
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
                app.load_selected_workspace_detail();
            }
            None
        }
        Message::SelectWorkspaceDown => {
            if app.selected_ws + 1 < app.workspaces.len() {
                app.selected_ws += 1;
                app.selected_repo = 0;
                app.load_selected_workspace_detail();
            }
            None
        }
        Message::SelectRepoUp => {
            if app.selected_repo > 0 {
                app.selected_repo -= 1;
            }
            None
        }
        Message::SelectRepoDown => {
            let max = app
                .selected_workspace()
                .map(|ws| ws.repos.len().saturating_sub(1))
                .unwrap_or(0);
            if app.selected_repo < max {
                app.selected_repo += 1;
            }
            None
        }
        Message::RefreshRepos => {
            let roots = app.config.repos.roots.clone();
            let depth = app.config.repos.max_depth;
            let repos = crate::core::repo::find_repos_in(&roots, depth);
            let _ = crate::core::repo::save_cache(&SpaceConfig::cache_path(), &repos);
            app.repos_cache = repos;
            app.status_message = Some(format!("Refreshed: {} repos found", app.repos_cache.len()));
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
fn build_branch_picker(
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

fn handle_create_key(app: &mut App, key: ratatui::crossterm::event::KeyEvent) {
    use crate::tui::screens::create::CreateStage;
    use ratatui::crossterm::event::KeyCode;

    // Extract the stage as a value so we don't hold a borrow on app.screen
    // while the match arms need to mutate it.
    let stage = {
        let Screen::CreateWorkspace(ref st) = app.screen else {
            return;
        };
        st.stage.clone()
    };

    match stage {
        CreateStage::PickRepos => {
            match key.code {
                KeyCode::Esc => {
                    app.screen = Screen::Dashboard;
                }
                KeyCode::Enter => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else {
                        return;
                    };
                    let confirmed: Vec<PathBuf> = st
                        .picker
                        .confirmed_items()
                        .into_iter()
                        .map(|i| i.full_path.clone())
                        .collect();
                    if confirmed.is_empty() {
                        st.error = Some("Select at least one repo".to_string());
                        return;
                    }
                    st.selected_repos = confirmed;
                    st.error = None;
                    st.stage = CreateStage::NameWorkspace;
                }
                KeyCode::Tab => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else {
                        return;
                    };
                    st.picker.toggle_highlighted();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else {
                        return;
                    };
                    st.picker.move_up();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else {
                        return;
                    };
                    st.picker.move_down();
                }
                KeyCode::Char('s')
                    if key
                        .modifiers
                        .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
                {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else {
                        return;
                    };
                    st.picker.cycle_scope();
                }
                _ => {
                    // All other keys (including plain 's') feed the picker input
                    let Screen::CreateWorkspace(ref mut st) = app.screen else {
                        return;
                    };
                    if let Some(req) = key_to_input_request(&key) {
                        st.picker.input.handle(req);
                    }
                    st.picker.refilter();
                }
            }
        }

        CreateStage::NameWorkspace => match key.code {
            KeyCode::Esc => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                st.stage = CreateStage::PickRepos;
            }
            KeyCode::Enter => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                let name = st.ws_name.value().trim().to_string();
                if name.is_empty() {
                    st.error = Some("Workspace name cannot be empty".to_string());
                    return;
                }
                st.error = None;
                st.stage = CreateStage::PickBranchStrategy;
            }
            _ => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                if let Some(req) = key_to_input_request(&key) {
                    st.ws_name.handle(req);
                }
            }
        },

        CreateStage::PickBranchStrategy => match key.code {
            KeyCode::Esc => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                st.stage = CreateStage::NameWorkspace;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                if st.branch_strategy_idx > 0 {
                    st.branch_strategy_idx -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                if st.branch_strategy_idx < 3 {
                    st.branch_strategy_idx += 1;
                }
            }
            KeyCode::Enter => {
                // If "Pick a branch..." selected, open branch picker
                let idx = {
                    let Screen::CreateWorkspace(ref st) = app.screen else {
                        return;
                    };
                    st.branch_strategy_idx
                };
                if idx == 3 {
                    // Build branch picker from the first selected repo
                    let (repo_path, repo_name) = {
                        let Screen::CreateWorkspace(ref st) = app.screen else {
                            return;
                        };
                        let path = st.selected_repos.first().cloned();
                        let name = path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        (path, name)
                    };
                    if let Some(repo_path) = repo_path {
                        match build_branch_picker(&repo_path, &repo_name) {
                            Some(picker) => {
                                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                                    return;
                                };
                                st.branch_picker = Some(picker);
                                st.stage = CreateStage::PickBranch;
                            }
                            None => {
                                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                                    return;
                                };
                                st.error =
                                    Some(format!("Could not list branches for {}", repo_name));
                            }
                        }
                    }
                } else {
                    do_create(app);
                }
            }
            _ => {}
        },

        CreateStage::PickBranch => match key.code {
            KeyCode::Esc => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                st.stage = CreateStage::PickBranchStrategy;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                if let Some(ref mut bp) = st.branch_picker {
                    bp.move_up();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                if let Some(ref mut bp) = st.branch_picker {
                    bp.move_down();
                }
            }
            KeyCode::Enter => {
                let picked = {
                    let Screen::CreateWorkspace(ref st) = app.screen else {
                        return;
                    };
                    st.branch_picker
                        .as_ref()
                        .and_then(|bp| bp.confirmed_items().into_iter().next())
                        .map(|item| item.name.clone())
                };
                if let Some(branch) = picked {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else {
                        return;
                    };
                    st.picked_branch = Some(branch);
                }
                do_create(app);
            }
            _ => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                if let Some(ref mut bp) = st.branch_picker {
                    if let Some(req) = key_to_input_request(&key) {
                        bp.input.handle(req);
                    }
                    bp.refilter();
                }
            }
        },

        CreateStage::Creating => {
            match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                    // Capture error message before switching screens
                    let error_msg = {
                        let Screen::CreateWorkspace(ref st) = app.screen else {
                            return;
                        };
                        st.error.clone()
                    };
                    app.screen = Screen::Dashboard;
                    if let Ok(ws) =
                        crate::core::workspace::list_workspaces(&app.config.workspaces.dir)
                    {
                        app.workspaces = ws;
                        app.selected_ws = 0;
                        app.load_selected_workspace_detail();
                    }
                    if let Some(err) = error_msg {
                        app.status_message = Some(format!("Create failed: {}", err));
                    }
                }
                _ => {}
            }
        }
    }
}

fn do_create(app: &mut App) {
    use crate::core::workspace::create_worktree;

    // Extract all needed data before mutating state, to satisfy borrow checker
    let (ws_name, strategy, repos, ws_dir) = {
        let Screen::CreateWorkspace(ref st) = app.screen else {
            return;
        };
        (
            st.ws_name.value().to_string(),
            st.branch_strategy(),
            st.selected_repos.clone(),
            app.config.workspaces.dir.clone(),
        )
    };

    // Transition to Creating stage
    {
        let Screen::CreateWorkspace(ref mut st) = app.screen else {
            return;
        };
        st.stage = crate::tui::screens::create::CreateStage::Creating;
        st.progress.clear();
        st.error = None;
    }

    for repo_path in &repos {
        let repo_name = repo_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "?".to_string());

        {
            let Screen::CreateWorkspace(ref mut st) = app.screen else {
                return;
            };
            st.progress
                .push(format!("Creating worktree for {}...", repo_name));
        }

        match create_worktree(repo_path, &ws_dir, &ws_name, &strategy) {
            Ok(_) => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                st.progress.push(format!("  \u{2713} {}", repo_name));
            }
            Err(e) => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else {
                    return;
                };
                st.progress.push(format!("  \u{2717} {}: {}", repo_name, e));
                st.error = Some(format!("Failed: {}", e));
            }
        }
    }

    // Check if there were errors
    let had_error = {
        let Screen::CreateWorkspace(ref st) = app.screen else {
            return;
        };
        st.error.is_some()
    };

    if !had_error {
        if let Ok(ws_list) = crate::core::workspace::list_workspaces(&ws_dir) {
            app.workspaces = ws_list;
            if let Some(idx) = app.workspaces.iter().position(|w| w.name == ws_name) {
                app.selected_ws = idx;
            }
            app.load_selected_workspace_detail();
        }
        app.screen = Screen::Dashboard;
        app.status_message = Some(format!("Created workspace '{}'", ws_name));
    }
    // If there was an error, stay on Creating stage so user can see the log
}

fn handle_go_key(app: &mut App, key: ratatui::crossterm::event::KeyEvent) {
    use ratatui::crossterm::event::KeyCode;

    match key.code {
        KeyCode::Esc => {
            app.screen = Screen::Dashboard;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let Screen::GoWorkspace(ref mut st) = app.screen else {
                return;
            };
            st.picker.move_up();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let Screen::GoWorkspace(ref mut st) = app.screen else {
                return;
            };
            st.picker.move_down();
        }
        KeyCode::Enter => {
            let target = {
                let Screen::GoWorkspace(ref st) = app.screen else {
                    return;
                };
                st.picker
                    .confirmed_items()
                    .into_iter()
                    .next()
                    .map(|i| i.full_path.clone())
            };
            if let Some(path) = target {
                app.space_cd_target = Some(path);
                app.should_quit = true;
            }
        }
        _ => {
            let Screen::GoWorkspace(ref mut st) = app.screen else {
                return;
            };
            if let Some(req) = key_to_input_request(&key) {
                st.picker.input.handle(req);
            }
            st.picker.refilter();
        }
    }
}

fn handle_add_key(app: &mut App, key: ratatui::crossterm::event::KeyEvent) {
    use crate::tui::screens::add::AddStage;
    use ratatui::crossterm::event::KeyCode;

    let stage = {
        let Screen::AddRepos(ref st) = app.screen else {
            return;
        };
        st.stage.clone()
    };

    match stage {
        AddStage::PickRepos => match key.code {
            KeyCode::Esc => {
                app.screen = Screen::Dashboard;
            }
            KeyCode::Enter => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                let confirmed: Vec<PathBuf> = st
                    .picker
                    .confirmed_items()
                    .into_iter()
                    .map(|i| i.full_path.clone())
                    .collect();
                if confirmed.is_empty() {
                    st.error = Some("Select at least one repo".to_string());
                    return;
                }
                st.selected_repos = confirmed;
                st.error = None;
                st.stage = AddStage::PickBranchStrategy;
            }
            KeyCode::Tab => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                st.picker.toggle_highlighted();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                st.picker.move_up();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                st.picker.move_down();
            }
            KeyCode::Char('s')
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
            {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                st.picker.cycle_scope();
            }
            _ => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                if let Some(req) = key_to_input_request(&key) {
                    st.picker.input.handle(req);
                }
                st.picker.refilter();
            }
        },

        AddStage::PickBranchStrategy => match key.code {
            KeyCode::Esc => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                st.stage = AddStage::PickRepos;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                if st.branch_strategy_idx > 0 {
                    st.branch_strategy_idx -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                if st.branch_strategy_idx < 3 {
                    st.branch_strategy_idx += 1;
                }
            }
            KeyCode::Enter => {
                let idx = {
                    let Screen::AddRepos(ref st) = app.screen else {
                        return;
                    };
                    st.branch_strategy_idx
                };
                if idx == 3 {
                    let (repo_path, repo_name) = {
                        let Screen::AddRepos(ref st) = app.screen else {
                            return;
                        };
                        let path = st.selected_repos.first().cloned();
                        let name = path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        (path, name)
                    };
                    if let Some(repo_path) = repo_path {
                        match build_branch_picker(&repo_path, &repo_name) {
                            Some(picker) => {
                                let Screen::AddRepos(ref mut st) = app.screen else {
                                    return;
                                };
                                st.branch_picker = Some(picker);
                                st.stage = AddStage::PickBranch;
                            }
                            None => {
                                let Screen::AddRepos(ref mut st) = app.screen else {
                                    return;
                                };
                                st.error =
                                    Some(format!("Could not list branches for {}", repo_name));
                            }
                        }
                    }
                } else {
                    do_add(app);
                }
            }
            _ => {}
        },

        AddStage::PickBranch => match key.code {
            KeyCode::Esc => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                st.stage = AddStage::PickBranchStrategy;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                if let Some(ref mut bp) = st.branch_picker {
                    bp.move_up();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                if let Some(ref mut bp) = st.branch_picker {
                    bp.move_down();
                }
            }
            KeyCode::Enter => {
                let picked = {
                    let Screen::AddRepos(ref st) = app.screen else {
                        return;
                    };
                    st.branch_picker
                        .as_ref()
                        .and_then(|bp| bp.confirmed_items().into_iter().next())
                        .map(|item| item.name.clone())
                };
                if let Some(branch) = picked {
                    let Screen::AddRepos(ref mut st) = app.screen else {
                        return;
                    };
                    st.picked_branch = Some(branch);
                }
                do_add(app);
            }
            _ => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                if let Some(ref mut bp) = st.branch_picker {
                    if let Some(req) = key_to_input_request(&key) {
                        bp.input.handle(req);
                    }
                    bp.refilter();
                }
            }
        },

        AddStage::Creating => match key.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                let error_msg = {
                    let Screen::AddRepos(ref st) = app.screen else {
                        return;
                    };
                    st.error.clone()
                };
                app.screen = Screen::Dashboard;
                if let Ok(ws) = crate::core::workspace::list_workspaces(&app.config.workspaces.dir)
                {
                    app.workspaces = ws;
                    app.selected_ws = 0;
                    app.load_selected_workspace_detail();
                }
                if let Some(err) = error_msg {
                    app.status_message = Some(format!("Add failed: {}", err));
                }
            }
            _ => {}
        },
    }
}

fn do_add(app: &mut App) {
    use crate::core::workspace::create_worktree;

    let (ws_name, strategy, repos, ws_dir) = {
        let Screen::AddRepos(ref st) = app.screen else {
            return;
        };
        (
            st.workspace_name.clone(),
            st.branch_strategy(),
            st.selected_repos.clone(),
            app.config.workspaces.dir.clone(),
        )
    };

    {
        let Screen::AddRepos(ref mut st) = app.screen else {
            return;
        };
        st.stage = crate::tui::screens::add::AddStage::Creating;
        st.progress.clear();
        st.error = None;
    }

    for repo_path in &repos {
        let repo_name = repo_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "?".to_string());

        {
            let Screen::AddRepos(ref mut st) = app.screen else {
                return;
            };
            st.progress
                .push(format!("Adding worktree for {}...", repo_name));
        }

        match create_worktree(repo_path, &ws_dir, &ws_name, &strategy) {
            Ok(_) => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                st.progress.push(format!("  \u{2713} {}", repo_name));
            }
            Err(e) => {
                let Screen::AddRepos(ref mut st) = app.screen else {
                    return;
                };
                st.progress.push(format!("  \u{2717} {}: {}", repo_name, e));
                st.error = Some(format!("Failed: {}", e));
            }
        }
    }

    let had_error = {
        let Screen::AddRepos(ref st) = app.screen else {
            return;
        };
        st.error.is_some()
    };

    if !had_error {
        if let Ok(ws_list) = crate::core::workspace::list_workspaces(&ws_dir) {
            app.workspaces = ws_list;
            if let Some(idx) = app.workspaces.iter().position(|w| w.name == ws_name) {
                app.selected_ws = idx;
            }
            app.load_selected_workspace_detail();
        }
        app.screen = Screen::Dashboard;
        app.status_message = Some(format!("Added repos to workspace '{}'", ws_name));
    }
    // If there was an error, stay on Creating stage so user can see the log
}

fn handle_delete_key(app: &mut App, key: ratatui::crossterm::event::KeyEvent) {
    use ratatui::crossterm::event::KeyCode;

    let (ws_name, ws_dir) = {
        let Screen::ConfirmDelete(ref st) = app.screen else {
            return;
        };
        (st.workspace_name.clone(), app.config.workspaces.dir.clone())
    };

    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            match crate::core::workspace::remove_workspace(&ws_dir, &ws_name, true) {
                Ok(()) => {
                    app.screen = Screen::Dashboard;
                    if let Ok(ws) = crate::core::workspace::list_workspaces(&ws_dir) {
                        app.workspaces = ws;
                        app.selected_ws = 0;
                    }
                    app.load_selected_workspace_detail();
                    app.status_message = Some(format!("Deleted workspace '{}'", ws_name));
                }
                Err(e) => {
                    app.screen = Screen::Dashboard;
                    app.status_message = Some(format!("Delete failed: {}", e));
                }
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.screen = Screen::Dashboard;
        }
        _ => {}
    }
}

fn handle_search_key(app: &mut App, key: ratatui::crossterm::event::KeyEvent) {
    use ratatui::crossterm::event::KeyCode;

    match key.code {
        KeyCode::Esc => {
            app.screen = Screen::Dashboard;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let Screen::RepoSearch(ref mut st) = app.screen else {
                return;
            };
            st.picker.move_up();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let Screen::RepoSearch(ref mut st) = app.screen else {
                return;
            };
            st.picker.move_down();
        }
        KeyCode::Enter => {
            // Get the selected repo name before any mutation
            let selected_name = {
                let Screen::RepoSearch(ref st) = app.screen else {
                    return;
                };
                st.picker
                    .confirmed_items()
                    .into_iter()
                    .next()
                    .map(|i| i.name.clone())
            };

            app.screen = Screen::Dashboard;

            if let Some(repo_name) = selected_name {
                // Walk workspaces to find one containing this repo
                let found_idx = app
                    .workspaces
                    .iter()
                    .position(|ws| ws.repos.iter().any(|r| r.name == repo_name));
                if let Some(idx) = found_idx {
                    app.selected_ws = idx;
                    app.selected_repo = 0;
                    app.load_selected_workspace_detail();
                } else {
                    app.status_message =
                        Some("Not in any workspace — use 'c' to create one".to_string());
                }
            }
        }
        _ => {
            let Screen::RepoSearch(ref mut st) = app.screen else {
                return;
            };
            if let Some(req) = key_to_input_request(&key) {
                st.picker.input.handle(req);
            }
            st.picker.refilter();
        }
    }
}

fn handle_config_key(app: &mut App, key: ratatui::crossterm::event::KeyEvent) {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    // Ctrl-S: commit any active edit, save, exit to dashboard
    if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
        if let Screen::ConfigEditor(ref mut st) = app.screen {
            if st.editing {
                st.commit_edit();
            }
        }
        let base_config = app.config.clone();
        let result = {
            let Screen::ConfigEditor(ref st) = app.screen else {
                return;
            };
            st.save_to_config(base_config)
        };
        match result {
            Ok(new_config) => {
                app.config = new_config;
                app.status_message = Some("Config saved".to_string());
            }
            Err(e) => {
                app.status_message = Some(format!("Save failed: {}", e));
            }
        }
        app.screen = Screen::Dashboard;
        return;
    }

    let editing = {
        let Screen::ConfigEditor(ref st) = app.screen else {
            return;
        };
        st.editing
    };

    if editing {
        match key.code {
            KeyCode::Esc => {
                let Screen::ConfigEditor(ref mut st) = app.screen else {
                    return;
                };
                st.cancel_edit();
            }
            KeyCode::Enter => {
                // Commit and advance to next field
                let Screen::ConfigEditor(ref mut st) = app.screen else {
                    return;
                };
                st.commit_edit();
                let next = (st.focused + 1).min(st.fields.len() - 1);
                st.focused = next;
            }
            _ => {
                let Screen::ConfigEditor(ref mut st) = app.screen else {
                    return;
                };
                if let Some(req) = key_to_input_request(&key) {
                    st.input.handle(req);
                }
            }
        }
    } else {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                // Exit without saving
                app.screen = Screen::Dashboard;
            }
            KeyCode::Enter => {
                let Screen::ConfigEditor(ref mut st) = app.screen else {
                    return;
                };
                st.start_editing();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let Screen::ConfigEditor(ref mut st) = app.screen else {
                    return;
                };
                if st.focused > 0 {
                    st.focused -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let Screen::ConfigEditor(ref mut st) = app.screen else {
                    return;
                };
                if st.focused + 1 < st.fields.len() {
                    st.focused += 1;
                }
            }
            _ => {}
        }
    }
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

    // Drain any stale input that accumulated before the TUI started —
    // e.g. keystrokes typed during a previous frozen/crashed session that
    // left the terminal in raw mode.  Without this, buffered events replay
    // immediately into the first field, corrupting it.
    while event::poll(std::time::Duration::ZERO)? {
        let _ = event::read()?;
    }

    enum ActiveScreen {
        Dashboard,
        Create,
        Go,
        Add,
        Delete,
        Search,
        Config,
    }

    loop {
        terminal.draw(|frame| crate::tui::ui::view(app, frame))?;

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Global: Ctrl-C always quits (raw mode swallows the OS signal)
                if key.code == KeyCode::Char('c')
                    && key.modifiers.contains(event::KeyModifiers::CONTROL)
                {
                    app.should_quit = true;
                    continue;
                }

                // Determine which screen is active without holding a borrow on app.screen,
                // so we can pass `&mut app` into the handler functions.
                let active = match &app.screen {
                    Screen::Dashboard => ActiveScreen::Dashboard,
                    Screen::CreateWorkspace(_) => ActiveScreen::Create,
                    Screen::GoWorkspace(_) => ActiveScreen::Go,
                    Screen::AddRepos(_) => ActiveScreen::Add,
                    Screen::ConfirmDelete(_) => ActiveScreen::Delete,
                    Screen::RepoSearch(_) => ActiveScreen::Search,
                    Screen::ConfigEditor(_) => ActiveScreen::Config,
                };

                let msg: Option<Message> = match active {
                    ActiveScreen::Create => {
                        handle_create_key(app, key);
                        None
                    }
                    ActiveScreen::Go => {
                        handle_go_key(app, key);
                        None
                    }
                    ActiveScreen::Add => {
                        handle_add_key(app, key);
                        None
                    }
                    ActiveScreen::Delete => {
                        handle_delete_key(app, key);
                        None
                    }
                    ActiveScreen::Search => {
                        handle_search_key(app, key);
                        None
                    }
                    ActiveScreen::Config => {
                        handle_config_key(app, key);
                        None
                    }
                    ActiveScreen::Dashboard => match (key.code, key.modifiers) {
                        (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => Some(Message::Quit),
                        (KeyCode::Tab, _) => Some(Message::FocusNext),
                        (KeyCode::Enter, _) => Some(Message::GoToWorkspace),
                        (KeyCode::Char('g'), _) => Some(Message::StartGo),
                        (KeyCode::Char('c'), _) => Some(Message::StartCreate),
                        (KeyCode::Char('a'), _) => Some(Message::StartAdd),
                        (KeyCode::Char('d'), _) => Some(Message::StartDelete),
                        (KeyCode::Char('r'), _) => Some(Message::RefreshRepos),
                        (KeyCode::Char('/'), _) => Some(Message::StartSearch),
                        (KeyCode::Char('S'), _) => Some(Message::StartConfig),
                        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => match app.focus {
                            Pane::Left => Some(Message::SelectWorkspaceUp),
                            Pane::Right => Some(Message::SelectRepoUp),
                        },
                        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => match app.focus {
                            Pane::Left => Some(Message::SelectWorkspaceDown),
                            Pane::Right => Some(Message::SelectRepoDown),
                        },
                        _ => None,
                    },
                };

                if let Some(m) = msg {
                    let mut next = update(app, m);
                    while let Some(m2) = next {
                        next = update(app, m2);
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

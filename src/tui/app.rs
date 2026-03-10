use crate::core::{config::SpaceConfig, workspace::{self, Workspace}};
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
pub enum Pane { Left, Right }

#[derive(Debug)]
pub enum Screen {
    Dashboard,
    CreateWorkspace(crate::tui::screens::create::CreateState),
}

#[derive(Debug)]
pub enum Message {
    Quit,
    FocusNext,
    SelectWorkspace(usize),
    SelectWorkspaceUp,
    SelectWorkspaceDown,
    SelectRepoUp,
    SelectRepoDown,
    GoToWorkspace,
    StartCreate,
    StartAdd,
    StartDelete,
    StartSearch,
    RefreshRepos,
    Tick,
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
        let repos_cache = crate::core::repo::load_cache(&SpaceConfig::cache_path())
            .unwrap_or_default();
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
        Message::Quit => { app.should_quit = true; None }
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
            if app.selected_repo > 0 { app.selected_repo -= 1; }
            None
        }
        Message::SelectRepoDown => {
            let max = app.selected_workspace()
                .map(|ws| ws.repos.len().saturating_sub(1))
                .unwrap_or(0);
            if app.selected_repo < max { app.selected_repo += 1; }
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
            let state = crate::tui::screens::create::CreateState::new(
                app.repos_cache.clone(),
                vec![],
            );
            app.screen = Screen::CreateWorkspace(state);
            None
        }
        // Stubbed — wired in Tasks 8-9
        Message::StartAdd | Message::StartDelete
        | Message::StartSearch | Message::SelectWorkspace(_) | Message::Tick => None,
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

fn handle_create_key(app: &mut App, key: ratatui::crossterm::event::KeyEvent) {
    use crate::tui::screens::create::CreateStage;
    use ratatui::crossterm::event::KeyCode;

    // Extract the stage as a value so we don't hold a borrow on app.screen
    // while the match arms need to mutate it.
    let stage = {
        let Screen::CreateWorkspace(ref st) = app.screen else { return; };
        st.stage.clone()
    };

    match stage {
        CreateStage::PickRepos => {
            match key.code {
                KeyCode::Esc => {
                    app.screen = Screen::Dashboard;
                }
                KeyCode::Enter => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
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
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    st.picker.toggle_highlighted();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    st.picker.move_up();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    st.picker.move_down();
                }
                KeyCode::Char('s')
                    if key
                        .modifiers
                        .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
                {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    st.picker.cycle_scope();
                }
                _ => {
                    // All other keys (including plain 's') feed the picker input
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    if let Some(req) = key_to_input_request(&key) {
                        st.picker.input.handle(req);
                    }
                    st.picker.refilter();
                }
            }
        }

        CreateStage::NameWorkspace => {
            match key.code {
                KeyCode::Esc => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    st.stage = CreateStage::PickRepos;
                }
                KeyCode::Enter => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    let name = st.ws_name.value().trim().to_string();
                    if name.is_empty() {
                        st.error = Some("Workspace name cannot be empty".to_string());
                        return;
                    }
                    st.error = None;
                    st.stage = CreateStage::PickBranchStrategy;
                }
                _ => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    if let Some(req) = key_to_input_request(&key) {
                        st.ws_name.handle(req);
                    }
                }
            }
        }

        CreateStage::PickBranchStrategy => {
            match key.code {
                KeyCode::Esc => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    st.stage = CreateStage::NameWorkspace;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    if st.branch_strategy_idx > 0 {
                        st.branch_strategy_idx -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                    if st.branch_strategy_idx < 2 {
                        st.branch_strategy_idx += 1;
                    }
                }
                KeyCode::Enter => {
                    do_create(app);
                }
                _ => {}
            }
        }

        CreateStage::Creating => {
            match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                    // Capture error message before switching screens
                    let error_msg = {
                        let Screen::CreateWorkspace(ref st) = app.screen else { return; };
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
        let Screen::CreateWorkspace(ref st) = app.screen else { return; };
        (
            st.ws_name.value().to_string(),
            st.branch_strategy(),
            st.selected_repos.clone(),
            app.config.workspaces.dir.clone(),
        )
    };

    // Transition to Creating stage
    {
        let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
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
            let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
            st.progress
                .push(format!("Creating worktree for {}...", repo_name));
        }

        match create_worktree(repo_path, &ws_dir, &ws_name, &strategy) {
            Ok(_) => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                st.progress.push(format!("  \u{2713} {}", repo_name));
            }
            Err(e) => {
                let Screen::CreateWorkspace(ref mut st) = app.screen else { return; };
                st.progress.push(format!("  \u{2717} {}: {}", repo_name, e));
                st.error = Some(format!("Failed: {}", e));
            }
        }
    }

    // Check if there were errors
    let had_error = {
        let Screen::CreateWorkspace(ref st) = app.screen else { return; };
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

fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
) -> Result<()> {
    use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

    loop {
        terminal.draw(|frame| crate::tui::ui::view(app, frame))?;

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                // Determine which screen is active without holding a borrow,
                // so we can pass `&mut app` into handle_create_key if needed.
                let is_create = matches!(app.screen, Screen::CreateWorkspace(_));

                let msg: Option<Message> = if is_create {
                    handle_create_key(app, key);
                    None
                } else {
                    // Screen::Dashboard
                    match (key.code, key.modifiers) {
                        (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => Some(Message::Quit),
                        (KeyCode::Tab, _) => Some(Message::FocusNext),
                        (KeyCode::Enter, _) => Some(Message::GoToWorkspace),
                        (KeyCode::Char('c'), _) => Some(Message::StartCreate),
                        (KeyCode::Char('a'), _) => Some(Message::StartAdd),
                        (KeyCode::Char('d'), _) => Some(Message::StartDelete),
                        (KeyCode::Char('r'), _) => Some(Message::RefreshRepos),
                        (KeyCode::Char('/'), _) => Some(Message::StartSearch),
                        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => match app.focus {
                            Pane::Left => Some(Message::SelectWorkspaceUp),
                            Pane::Right => Some(Message::SelectRepoUp),
                        },
                        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => match app.focus {
                            Pane::Left => Some(Message::SelectWorkspaceDown),
                            Pane::Right => Some(Message::SelectRepoDown),
                        },
                        _ => None,
                    }
                };

                if let Some(m) = msg {
                    let mut next = update(app, m);
                    while let Some(m2) = next {
                        next = update(app, m2);
                    }
                }
            }
        }

        if app.should_quit { break; }
    }
    Ok(())
}

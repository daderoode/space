use crate::core::{config::SpaceConfig, workspace::{self, Workspace}};
use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pane { Left, Right }

#[derive(Debug)]
pub enum Screen {
    Dashboard,
    // Populated in Tasks 7-9:
    // CreateWorkspace(CreateState),
    // AddRepos(AddState),
    // GoWorkspace,
    // ConfirmDelete(String),
    // ConfigEditor,
    // RepoSearch,
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
        Ok(Self {
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
        })
    }

    pub fn selected_workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.selected_ws)
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
            if app.selected_ws > 0 { app.selected_ws -= 1; }
            app.selected_repo = 0;
            None
        }
        Message::SelectWorkspaceDown => {
            if app.selected_ws + 1 < app.workspaces.len() { app.selected_ws += 1; }
            app.selected_repo = 0;
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
        // Stubbed — wired in Tasks 7-9
        Message::StartCreate | Message::StartAdd | Message::StartDelete
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
                let msg = match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => Some(Message::Quit),
                    (KeyCode::Tab, _) => Some(Message::FocusNext),
                    (KeyCode::Enter, _) => Some(Message::GoToWorkspace),
                    (KeyCode::Char('c'), _) => Some(Message::StartCreate),
                    (KeyCode::Char('a'), _) => Some(Message::StartAdd),
                    (KeyCode::Char('d'), _) => Some(Message::StartDelete),
                    (KeyCode::Char('r'), _) => Some(Message::RefreshRepos),
                    (KeyCode::Char('/'), _) => Some(Message::StartSearch),
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        match app.focus {
                            Pane::Left => Some(Message::SelectWorkspaceUp),
                            Pane::Right => Some(Message::SelectRepoUp),
                        }
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        match app.focus {
                            Pane::Left => Some(Message::SelectWorkspaceDown),
                            Pane::Right => Some(Message::SelectRepoDown),
                        }
                    }
                    _ => None,
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

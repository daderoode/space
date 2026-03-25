use crate::tui::actions::{ScreenAction, ScreenContext};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug)]
pub struct DeleteState {
    pub workspace_name: String,
    pub repo_names: Vec<String>,
}

impl DeleteState {
    pub fn handle_key(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => ScreenAction::DeleteWorkspace {
                name: self.workspace_name.clone(),
                force: true,
            },
            KeyCode::Char('n') | KeyCode::Esc => ScreenAction::Back,
            _ => ScreenAction::Continue,
        }
    }
}

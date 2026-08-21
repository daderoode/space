use crate::tui::actions::{ScreenAction, ScreenContext};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug)]
pub struct DeleteState {
    pub workspace_name: String,
    pub repo_names: Vec<String>,
}

impl DeleteState {
    /// Default-No confirmation, matching `ConfirmPush` and `RebaseConfirm`:
    /// only an explicit `y`/`Y` deletes. `Enter` declines, so a reflex Enter
    /// can never destroy a workspace.
    pub fn handle_key(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => ScreenAction::DeleteWorkspace {
                name: self.workspace_name.clone(),
                force: true,
            },
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Enter | KeyCode::Esc => {
                ScreenAction::Back
            }
            _ => ScreenAction::Continue,
        }
    }
}

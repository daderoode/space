use super::default_no_confirm;
use crate::tui::actions::{ScreenAction, ScreenContext};
use ratatui::crossterm::event::KeyEvent;

#[derive(Debug)]
pub struct DeleteState {
    pub workspace_name: String,
    pub repo_names: Vec<String>,
}

impl DeleteState {
    /// Default-No confirmation shared with `ConfirmPush` and `RebaseConfirm`:
    /// only an explicit `y`/`Y` deletes, so a reflex Enter can never destroy
    /// a workspace.
    pub fn handle_key(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        // Nothing is typed here, so `?` always opens help.
        if super::opens_help(key.code, true) {
            return ScreenAction::OpenHelp;
        }
        match default_no_confirm(key.code) {
            Some(true) => ScreenAction::DeleteWorkspace {
                name: self.workspace_name.clone(),
                force: true,
            },
            Some(false) => ScreenAction::Back,
            None => ScreenAction::Continue,
        }
    }
}

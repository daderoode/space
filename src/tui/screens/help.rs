use crate::tui::actions::{ScreenAction, ScreenContext};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

pub fn handle_key(key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => ScreenAction::Back,
        _ => ScreenAction::Continue,
    }
}

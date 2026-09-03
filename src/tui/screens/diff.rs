use crate::core::git::FileDiff;
use crate::tui::actions::{ScreenAction, ScreenContext};
use crate::tui::screens::sync_report::PAGE_ROWS;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;

#[derive(Debug)]
pub struct DiffViewerState {
    pub repo_index: usize,
    pub repo_name: String,
    pub repo_path: PathBuf,
    pub file_path: String,
    pub staged: bool,
    pub diff: Result<FileDiff, String>,
    pub scroll_offset: u16,
    pub total_lines: u16,
}

impl DiffViewerState {
    pub fn handle_key(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => ScreenAction::Back,
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                ScreenAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_offset = self
                    .scroll_offset
                    .saturating_add(1)
                    .min(self.total_lines.saturating_sub(1));
                ScreenAction::Continue
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(PAGE_ROWS as u16);
                ScreenAction::Continue
            }
            KeyCode::PageDown => {
                self.scroll_offset = self
                    .scroll_offset
                    .saturating_add(PAGE_ROWS as u16)
                    .min(self.total_lines.saturating_sub(1));
                ScreenAction::Continue
            }
            KeyCode::Home => {
                self.scroll_offset = 0;
                ScreenAction::Continue
            }
            KeyCode::End => {
                self.scroll_offset = self.total_lines.saturating_sub(1);
                ScreenAction::Continue
            }
            // Nothing is typed here, so `?` always opens help.
            c if crate::tui::screens::opens_help(c, true) => ScreenAction::OpenHelp,
            KeyCode::Char('s') | KeyCode::Char(' ') => ScreenAction::StageFile {
                repo_index: self.repo_index,
                repo_path: self.repo_path.clone(),
                path: self.file_path.clone(),
                currently_staged: self.staged,
            },
            _ => ScreenAction::Continue,
        }
    }
}

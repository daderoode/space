use crate::tui::actions::{ScreenAction, ScreenContext};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;

/// Stage of the git-operations overlay. Phase 1 ships only the action menu;
/// later phases add Committing / Log / Running / ConfirmPush.
#[derive(Debug, Clone, PartialEq)]
pub enum GitOpsStage {
    Menu,
}

pub struct GitOpsState {
    pub stage: GitOpsStage,
    pub repo_name: String,
    // Retained for the network/commit sub-flows added in later phases.
    #[allow(dead_code)]
    pub repo_path: PathBuf,
    pub branch: String,
    pub selected: usize,
    pub has_staged: bool,
    pub status: Option<String>,
}

impl std::fmt::Debug for GitOpsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitOpsState")
            .field("stage", &self.stage)
            .field("repo_name", &self.repo_name)
            .field("branch", &self.branch)
            .field("selected", &self.selected)
            .finish()
    }
}

impl GitOpsState {
    /// Build the menu state for a repo. Panic-safe on non-existent paths:
    /// `current_branch` and `file_diff` failures degrade to "?" / no-staged.
    pub fn new(repo_name: String, repo_path: PathBuf) -> Self {
        let branch =
            crate::core::git::current_branch(&repo_path).unwrap_or_else(|_| "?".to_string());
        let has_staged = crate::core::git::file_diff(&repo_path)
            .map(|v| v.iter().any(|e| e.staged))
            .unwrap_or(false);
        Self {
            stage: GitOpsStage::Menu,
            repo_name,
            repo_path,
            branch,
            selected: 0,
            has_staged,
            status: None,
        }
    }

    /// Highest selectable menu index (six items, 0..=5).
    const MAX_IDX: usize = 5;

    pub fn handle_key(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => ScreenAction::Back,
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected < Self::MAX_IDX {
                    self.selected += 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                self.fire(self.selected);
                ScreenAction::Continue
            }
            KeyCode::Char('f') => {
                self.fire(0);
                ScreenAction::Continue
            }
            KeyCode::Char('p') => {
                self.fire(1);
                ScreenAction::Continue
            }
            KeyCode::Char('P') => {
                self.fire(2);
                ScreenAction::Continue
            }
            KeyCode::Char('c') => {
                self.fire(3);
                ScreenAction::Continue
            }
            KeyCode::Char('l') => {
                self.fire(4);
                ScreenAction::Continue
            }
            KeyCode::Char('r') => {
                self.fire(5);
                ScreenAction::Continue
            }
            _ => ScreenAction::Continue,
        }
    }

    /// Fire the menu item at `idx`, moving the highlight to it and setting a
    /// placeholder status. Real sub-flows land in later phases; every action
    /// stays inside the menu.
    fn fire(&mut self, idx: usize) {
        self.selected = idx;
        let msg = match idx {
            0 => "Fetch: not yet implemented",
            1 => "Pull: not yet implemented",
            2 => "Push: not yet implemented",
            3 if !self.has_staged => "Stage files first with s/S",
            3 => "Commit: not yet implemented",
            4 => "Log: not yet implemented",
            5 => "Rebase coming soon (item 7)",
            _ => return,
        };
        self.status = Some(msg.to_string());
    }
}

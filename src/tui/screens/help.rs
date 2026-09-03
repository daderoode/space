use crate::tui::app::{Pane, Screen};
use crate::tui::keybindings;
use crate::tui::screens::sync_report::PAGE_ROWS;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::cell::Cell;

/// State of the help overlay.
///
/// Help is a layer drawn over whatever screen is showing, never a screen of
/// its own: `poll_sync_result` and `poll_gitop_result` gate on the current
/// `Screen` variant and cancel their worker when it does not match, so moving
/// a mid-flow screen into a help variant would kill the work it was showing.
/// See `docs/adr/0001-help-is-an-overlay-layer-not-a-screen.md`.
#[derive(Debug)]
pub struct HelpState {
    /// First registry row shown, in rendered lines.
    pub scroll: usize,
    /// Rows the overlay could show the last time it was drawn. Recorded by the
    /// renderer so a key press starts from what was actually on screen, the
    /// same trick `LogView` uses for the Creating log.
    viewport: Cell<usize>,
}

impl HelpState {
    /// Open scrolled to the first row of `group`, so help reached from a flow
    /// lands on that flow's keys instead of at the top of the list.
    pub fn opening_at(group: &str) -> Self {
        HelpState {
            scroll: keybindings::group_row_offset(group),
            viewport: Cell::new(0),
        }
    }

    /// The first line to show for `total` lines in `visible` rows this frame.
    /// Clamps so the last screenful is the end rather than blank rows.
    pub fn offset(&self, total: usize, visible: usize) -> usize {
        self.viewport.set(visible);
        self.scroll.min(total.saturating_sub(visible))
    }

    /// Scroll keys for a registry of `total` rendered lines. Returns true when
    /// the key closes the overlay.
    pub fn handle_key(&mut self, key: KeyEvent, total: usize) -> bool {
        let visible = self.viewport.get().max(1);
        let max = total.saturating_sub(visible);
        let current = self.scroll.min(max);
        let target = match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::F(1) => return true,
            KeyCode::Up | KeyCode::Char('k') => current.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => current.saturating_add(1),
            KeyCode::PageUp => current.saturating_sub(PAGE_ROWS),
            KeyCode::PageDown => current.saturating_add(PAGE_ROWS),
            KeyCode::Home => 0,
            KeyCode::End => max,
            // Everything else is swallowed: the overlay is modal, so no key
            // reaches the screen beneath it.
            _ => return false,
        };
        self.scroll = target.min(max);
        false
    }
}

/// The registry group help should open on, given the screen it was reached
/// from. The group order itself never changes; only the landing row does.
pub fn landing_group(screen: &Screen, focus: Pane) -> &'static str {
    use crate::tui::screens::{add::AddStage, create::CreateStage};
    match screen {
        Screen::Dashboard => match focus {
            Pane::Left => "Workspace Pane",
            Pane::Right => "Repo Pane",
        },
        Screen::CreateWorkspace(st) => match st.stage {
            CreateStage::PickRepos => "Repo Picker",
            CreateStage::Syncing => "Sync Report",
            _ => "Create / Add Flow",
        },
        Screen::AddRepos(st) => match st.stage {
            AddStage::PickRepos => "Repo Picker",
            AddStage::Syncing => "Sync Report",
            _ => "Create / Add Flow",
        },
        Screen::GoWorkspace(_) | Screen::FilterWorkspace(_) | Screen::RepoSearch(_) => {
            "Space & Repo Pickers"
        }
        Screen::ConfirmDelete(_) => "Delete Confirm",
        Screen::ConfigEditor(_) => "Config Editor",
        Screen::DiffViewer(_) => "Diff Viewer",
        Screen::SwitchBranch(_) => "Switch Branch",
        Screen::GitOps(_) => "Git Operations",
    }
}

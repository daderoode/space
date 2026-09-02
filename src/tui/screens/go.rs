use crate::tui::actions::{ScreenAction, ScreenContext};
use crate::tui::widgets::fuzzy_picker::{FuzzyPicker, PickerItem};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// What Enter does in a space picker. `g` (go) and the space filter share
/// `GoState` and its key handling; only the confirm action differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Go: cd into the chosen space's directory and quit the TUI.
    CdAndQuit,
    /// Space filter: select the chosen space in place and stay in the dashboard.
    SelectInPlace,
}

pub struct GoState {
    pub picker: FuzzyPicker,
    pub confirm: ConfirmAction,
}

impl GoState {
    /// The `g` picker: Enter cds into the space and quits.
    pub fn new(workspaces: &[crate::core::workspace::Workspace]) -> Self {
        Self::with_confirm(
            "Go to workspace  ENTER=go  ESC=cancel",
            workspaces,
            ConfirmAction::CdAndQuit,
        )
    }

    /// The space filter: Enter selects the space in place.
    pub fn filter(workspaces: &[crate::core::workspace::Workspace]) -> Self {
        Self::with_confirm(
            "Filter spaces  ENTER=select  ESC=cancel",
            workspaces,
            ConfirmAction::SelectInPlace,
        )
    }

    /// Rows are name plus repo count. The count sits in the `branch` slot
    /// (branch colour, excluded from matching) and `parent` stays empty so a
    /// literal like "workspaces" never matches a query. Item order equals
    /// workspace order, so a picker index is a space index. The count comes
    /// from a scan of the space's own directory (the same fast scan the repos
    /// pane uses for skeletons) because the dashboard only loads repos for
    /// the selected space.
    fn with_confirm(
        prompt: &str,
        workspaces: &[crate::core::workspace::Workspace],
        confirm: ConfirmAction,
    ) -> Self {
        let items: Vec<PickerItem> = workspaces
            .iter()
            .map(|ws| {
                let count = ws
                    .path
                    .parent()
                    .map(|dir| {
                        crate::core::workspace::workspace_repo_skeletons(dir, &ws.name).len()
                    })
                    .unwrap_or(0);
                let label = if count == 1 {
                    "1 repo".to_string()
                } else {
                    format!("{} repos", count)
                };
                PickerItem {
                    name: ws.name.clone(),
                    parent: String::new(),
                    full_path: ws.path.clone(),
                    branch: Some(label),
                    remote_url: None,
                }
            })
            .collect();
        GoState {
            picker: FuzzyPicker::new(prompt, items, false),
            confirm,
        }
    }

    /// Arrows move the highlight; every other key edits the query (letters
    /// are text in a typed picker, so there is no `j`/`k` navigation here).
    pub fn handle_key(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        match key.code {
            KeyCode::Esc => ScreenAction::Back,
            KeyCode::Up => {
                self.picker.move_up();
                ScreenAction::Continue
            }
            KeyCode::Down => {
                self.picker.move_down();
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                let Some(&idx) = self.picker.filtered.get(self.picker.highlighted) else {
                    return ScreenAction::Continue;
                };
                match self.confirm {
                    ConfirmAction::CdAndQuit => {
                        ScreenAction::CdAndQuit(self.picker.all_items[idx].full_path.clone())
                    }
                    ConfirmAction::SelectInPlace => ScreenAction::SelectWorkspace(idx),
                }
            }
            _ => {
                if let Some(req) = crate::tui::app::key_to_input_request(&key) {
                    self.picker.input.handle(req);
                }
                self.picker.refilter();
                ScreenAction::Continue
            }
        }
    }
}

impl std::fmt::Debug for GoState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoState")
            .field("confirm", &self.confirm)
            .finish()
    }
}

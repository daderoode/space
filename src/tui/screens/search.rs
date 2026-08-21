use crate::tui::actions::{ScreenAction, ScreenContext};
use crate::tui::widgets::fuzzy_picker::{FuzzyPicker, PickerItem};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

pub struct SearchState {
    pub picker: FuzzyPicker,
}

impl SearchState {
    pub fn new(repos: Vec<std::path::PathBuf>) -> Self {
        let items: Vec<PickerItem> = repos.into_iter().map(PickerItem::from_path).collect();
        SearchState {
            picker: FuzzyPicker::new("Search repos  ENTER=navigate  ESC=cancel", items, false),
        }
    }

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
            // j/k navigate only while the query is empty; once the user is
            // typing they are literal characters (a repo named "jackal" must
            // be reachable). Arrows always navigate.
            KeyCode::Char('k') if self.picker.input.value().is_empty() => {
                self.picker.move_up();
                ScreenAction::Continue
            }
            KeyCode::Char('j') if self.picker.input.value().is_empty() => {
                self.picker.move_down();
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                if let Some(item) = self.picker.confirmed_items().into_iter().next() {
                    ScreenAction::NavigateToWorkspace(item.name.clone())
                } else {
                    ScreenAction::Continue
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

impl std::fmt::Debug for SearchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SearchState")
    }
}

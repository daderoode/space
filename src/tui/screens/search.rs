use crate::tui::widgets::fuzzy_picker::{FuzzyPicker, PickerItem};

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
}

impl std::fmt::Debug for SearchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SearchState")
    }
}

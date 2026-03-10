use nucleo::{Config as NucleoConfig, Utf32Str};
use nucleo::pattern::{CaseMatching, Normalization, AtomKind, Pattern};
use std::collections::HashSet;
use std::path::PathBuf;
use tui_input::Input;

#[derive(Debug, Clone)]
pub struct PickerItem {
    pub name: String,
    pub parent: String,
    pub full_path: PathBuf,
}

impl PickerItem {
    pub fn from_path(path: PathBuf) -> Self {
        let name = path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = path.parent()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self { name, parent, full_path: path }
    }
}

pub struct FuzzyPicker {
    pub prompt: String,
    pub input: Input,
    pub all_items: Vec<PickerItem>,
    pub filtered: Vec<usize>,      // indices into all_items, sorted by score
    pub highlighted: usize,        // index into filtered
    pub toggled: HashSet<usize>,   // indices into all_items
    pub multi: bool,
    pub scope: Option<String>,
    pub available_scopes: Vec<String>,
    pub scope_idx: usize,
}

impl FuzzyPicker {
    pub fn new(prompt: impl Into<String>, items: Vec<PickerItem>, multi: bool) -> Self {
        // Collect unique parent dir names as available scopes
        let mut scopes: Vec<String> = items.iter()
            .map(|i| i.parent.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        scopes.sort();

        let len = items.len();
        let mut picker = Self {
            prompt: prompt.into(),
            input: Input::default(),
            all_items: items,
            filtered: (0..len).collect(),
            highlighted: 0,
            toggled: HashSet::new(),
            multi,
            scope: None,
            available_scopes: scopes,
            scope_idx: 0,
        };
        picker.refilter();
        picker
    }

    pub fn query(&self) -> &str {
        self.input.value()
    }

    /// Returns the query stripped of any scope prefix (text after last `/`)
    fn fuzzy_query(&self) -> &str {
        let q = self.input.value();
        if let Some(pos) = q.rfind('/') {
            &q[pos + 1..]
        } else {
            q
        }
    }

    /// Extract scope from query (text before and including last `/`)
    fn query_scope(&self) -> Option<String> {
        let q = self.input.value();
        if let Some(pos) = q.rfind('/') {
            let scope_part = &q[..pos];
            if !scope_part.is_empty() {
                // Strip any leading path components to get just the dir name
                let scope_dir = scope_part.rsplit('/').next().unwrap_or(scope_part);
                return Some(scope_dir.to_string());
            }
        }
        None
    }

    pub fn refilter(&mut self) {
        // Determine effective scope: query-derived or Ctrl-S scope
        let effective_scope = self.query_scope().or_else(|| self.scope.clone());

        let query = self.fuzzy_query().to_string();

        // Pre-filter by scope
        let scoped: Vec<usize> = (0..self.all_items.len())
            .filter(|&i| {
                if let Some(ref s) = effective_scope {
                    self.all_items[i].parent.to_lowercase().contains(&s.to_lowercase())
                        || self.all_items[i].full_path.to_string_lossy()
                            .to_lowercase().contains(&format!("/{}/", s.to_lowercase()))
                } else {
                    true
                }
            })
            .collect();

        if query.is_empty() {
            self.filtered = scoped;
        } else {
            let mut matcher = nucleo::Matcher::new(NucleoConfig::DEFAULT);
            let pattern = Pattern::new(
                &query,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
            );

            let mut scored: Vec<(u32, usize)> = scoped.iter()
                .filter_map(|&i| {
                    let item = &self.all_items[i];
                    // Match against "name (parent)" combined string
                    let display = format!("{} {}", item.name, item.parent);
                    let mut buf = Vec::new();
                    let haystack = Utf32Str::new(&display, &mut buf);
                    pattern.score(haystack, &mut matcher).map(|s| (s, i))
                })
                .collect();

            scored.sort_by(|a, b| b.0.cmp(&a.0));
            self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        }

        // Clamp highlight
        if self.highlighted >= self.filtered.len() {
            self.highlighted = self.filtered.len().saturating_sub(1);
        }
    }

    pub fn cycle_scope(&mut self) {
        if self.available_scopes.is_empty() { return; }
        self.scope_idx = (self.scope_idx + 1) % (self.available_scopes.len() + 1);
        self.scope = if self.scope_idx == 0 {
            None
        } else {
            Some(self.available_scopes[self.scope_idx - 1].clone())
        };
        self.refilter();
    }

    pub fn toggle_highlighted(&mut self) {
        if let Some(&item_idx) = self.filtered.get(self.highlighted) {
            if self.toggled.contains(&item_idx) {
                self.toggled.remove(&item_idx);
            } else {
                self.toggled.insert(item_idx);
            }
        }
    }

    pub fn confirmed_items(&self) -> Vec<&PickerItem> {
        if self.multi {
            if self.toggled.is_empty() {
                // If nothing toggled, return highlighted item
                self.filtered.get(self.highlighted)
                    .map(|&i| vec![&self.all_items[i]])
                    .unwrap_or_default()
            } else {
                let mut items: Vec<&PickerItem> = self.toggled.iter()
                    .map(|&i| &self.all_items[i])
                    .collect();
                items.sort_by_key(|i| &i.name);
                items
            }
        } else {
            self.filtered.get(self.highlighted)
                .map(|&i| vec![&self.all_items[i]])
                .unwrap_or_default()
        }
    }

    pub fn move_up(&mut self) {
        if self.highlighted > 0 { self.highlighted -= 1; }
    }

    pub fn move_down(&mut self) {
        if self.highlighted + 1 < self.filtered.len() { self.highlighted += 1; }
    }
}

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

/// Render the picker as a centered overlay popup.
pub fn render(picker: &FuzzyPicker, frame: &mut Frame) {
    let area = centered_rect(70, 60, frame.area());

    // Clear background
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(format!(" {} ", picker.prompt));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Check if scope is active
    let eff_scope: Option<String> = {
        let q = picker.input.value();
        if let Some(pos) = q.rfind('/') {
            let scope_part = &q[..pos];
            if !scope_part.is_empty() {
                let scope_dir = scope_part.rsplit('/').next().unwrap_or(scope_part);
                Some(scope_dir.to_string())
            } else { None }
        } else { None }
    }.or_else(|| picker.scope.clone());
    let show_scope = eff_scope.is_some();
    let scope_height = if show_scope { 1u16 } else { 0u16 };

    let sections = Layout::vertical([
        Constraint::Length(1),                          // input
        Constraint::Length(scope_height),               // scope line
        Constraint::Min(3),                             // list
        Constraint::Length(1),                          // status line
    ])
    .split(inner);

    // Input line
    let input_text = format!("> {}", picker.input.value());
    frame.render_widget(
        Paragraph::new(input_text).style(Style::default().fg(Color::White)),
        sections[0],
    );

    // Scope line
    if show_scope {
        let eff = eff_scope.unwrap_or_default();
        let scope_line = Line::from(vec![
            Span::styled(format!("  scope: {}/", eff), Style::default().fg(Color::DarkGray)),
            Span::styled("  CTRL-S: cycle", Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)),
        ]);
        frame.render_widget(Paragraph::new(scope_line), sections[1]);
    }

    // Items list
    let visible_items: Vec<ListItem> = picker.filtered.iter()
        .map(|&i| {
            let item = &picker.all_items[i];
            let toggled = picker.toggled.contains(&i);
            let dot = if toggled { "● " } else { "  " };
            let line = Line::from(vec![
                Span::styled(dot, Style::default().fg(Color::Cyan)),
                Span::raw(item.name.clone()),
                Span::styled(
                    format!("  ({})", item.parent),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(visible_items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut list_state = ListState::default();
    if !picker.filtered.is_empty() {
        list_state.select(Some(picker.highlighted));
    }
    frame.render_stateful_widget(list, sections[2], &mut list_state);

    // Status line
    let status = Line::from(vec![
        Span::styled(
            format!("{} selected", picker.toggled.len()),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{}/{} matched", picker.filtered.len(), picker.all_items.len()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(status), sections[3]);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_items(paths: &[&str]) -> Vec<PickerItem> {
        paths.iter().map(|p| PickerItem::from_path(PathBuf::from(p))).collect()
    }

    #[test]
    fn filters_by_query() {
        let items = make_items(&["/work/zss/vgateway-auth", "/work/omari/omari-api"]);
        let mut picker = FuzzyPicker::new("test", items, false);
        picker.input = picker.input.with_value("vgate".into());
        picker.refilter();
        assert_eq!(picker.filtered.len(), 1);
        assert_eq!(picker.all_items[picker.filtered[0]].name, "vgateway-auth");
    }

    #[test]
    fn scope_filters_by_parent() {
        let items = make_items(&[
            "/work/zss/vgateway-auth",
            "/work/omari/omari-api",
            "/work/zss/vgateway-transaction",
        ]);
        let mut picker = FuzzyPicker::new("test", items, false);
        picker.input = picker.input.with_value("zss/".into());
        picker.refilter();
        assert_eq!(picker.filtered.len(), 2);
        assert!(picker.all_items[picker.filtered[0]].parent == "zss"
            || picker.all_items[picker.filtered[1]].parent == "zss");
    }

    #[test]
    fn multi_select_toggle() {
        let items = make_items(&["/work/zss/a", "/work/zss/b"]);
        let mut picker = FuzzyPicker::new("test", items, true);
        picker.toggle_highlighted();
        assert_eq!(picker.toggled.len(), 1);
        picker.toggle_highlighted();
        assert_eq!(picker.toggled.len(), 0);
    }
}

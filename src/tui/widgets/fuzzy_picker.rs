use nucleo::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo::{Config as NucleoConfig, Utf32Str};
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
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            name,
            parent,
            full_path: path,
        }
    }
}

pub struct FuzzyPicker {
    pub prompt: String,
    pub input: Input,
    pub all_items: Vec<PickerItem>,
    pub filtered: Vec<usize>,    // indices into all_items, sorted by score
    pub highlighted: usize,      // index into filtered
    pub toggled: HashSet<usize>, // indices into all_items
    pub multi: bool,
    pub scope: Option<String>,
    pub available_scopes: Vec<String>,
    pub scope_idx: usize,
    pub match_indices: Vec<Vec<u32>>, // parallel to `filtered` — match char positions per item
}

impl FuzzyPicker {
    pub fn new(prompt: impl Into<String>, items: Vec<PickerItem>, multi: bool) -> Self {
        // Collect unique parent dir names as available scopes
        let mut scopes: Vec<String> = items
            .iter()
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
            match_indices: vec![],
        };
        picker.refilter();
        picker
    }

    #[allow(dead_code)]
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
    pub fn query_scope(&self) -> Option<String> {
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
                    self.all_items[i]
                        .parent
                        .to_lowercase()
                        .contains(&s.to_lowercase())
                        || self.all_items[i]
                            .full_path
                            .to_string_lossy()
                            .to_lowercase()
                            .contains(&format!("/{}/", s.to_lowercase()))
                } else {
                    true
                }
            })
            .collect();

        if query.is_empty() {
            self.match_indices = vec![vec![]; scoped.len()];
            self.filtered = scoped;
        } else {
            let mut matcher = nucleo::Matcher::new(NucleoConfig::DEFAULT);
            let pattern = Pattern::new(
                &query,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
            );

            let mut scored: Vec<(u32, usize, Vec<u32>)> = scoped
                .iter()
                .filter_map(|&i| {
                    let item = &self.all_items[i];
                    // Match against "name parent" combined string
                    let display = format!("{} {}", item.name, item.parent);
                    let mut buf = Vec::new();
                    let haystack = Utf32Str::new(&display, &mut buf);
                    let score = pattern.score(haystack, &mut matcher)?;
                    let mut indices: Vec<u32> = Vec::new();
                    pattern.indices(haystack, &mut matcher, &mut indices);
                    indices.sort_unstable();
                    indices.dedup();
                    Some((score, i, indices))
                })
                .collect();

            scored.sort_by(|a, b| b.0.cmp(&a.0));
            self.filtered = scored.iter().map(|(_, i, _)| *i).collect();
            self.match_indices = scored.into_iter().map(|(_, _, idx)| idx).collect();
        }

        // Clamp highlight
        if self.highlighted >= self.filtered.len() {
            self.highlighted = self.filtered.len().saturating_sub(1);
        }
    }

    pub fn cycle_scope(&mut self) {
        if self.available_scopes.is_empty() {
            return;
        }
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
                self.filtered
                    .get(self.highlighted)
                    .map(|&i| vec![&self.all_items[i]])
                    .unwrap_or_default()
            } else {
                let mut items: Vec<&PickerItem> =
                    self.toggled.iter().map(|&i| &self.all_items[i]).collect();
                items.sort_by_key(|i| &i.name);
                items
            }
        } else {
            self.filtered
                .get(self.highlighted)
                .map(|&i| vec![&self.all_items[i]])
                .unwrap_or_default()
        }
    }

    pub fn move_up(&mut self) {
        if self.highlighted > 0 {
            self.highlighted -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.highlighted + 1 < self.filtered.len() {
            self.highlighted += 1;
        }
    }
}

use crate::tui::theme;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

/// Build a list of styled spans for `text`, highlighting characters whose byte
/// positions appear in `indices` with `highlight` and the rest with `normal`.
fn build_highlighted_spans<'a>(
    text: &'a str,
    indices: &[u32],
    normal: Style,
    highlight: Style,
) -> Vec<Span<'a>> {
    let match_set: HashSet<usize> = indices
        .iter()
        .map(|&i| i as usize)
        .filter(|&i| i < text.len())
        .collect();

    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut current = String::new();
    let mut current_is_match = false;

    for (byte_idx, ch) in text.char_indices() {
        let is_match = match_set.contains(&byte_idx);
        if is_match != current_is_match && !current.is_empty() {
            let style = if current_is_match { highlight } else { normal };
            spans.push(Span::styled(current.clone(), style));
            current.clear();
        }
        current_is_match = is_match;
        current.push(ch);
    }
    if !current.is_empty() {
        let style = if current_is_match { highlight } else { normal };
        spans.push(Span::styled(current, style));
    }
    spans
}

/// Render the picker as a centered overlay popup.
pub fn render(picker: &FuzzyPicker, frame: &mut Frame) {
    let area = centered_rect(70, 60, frame.area());

    // Clear background
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(format!(" {} ", picker.prompt));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Check if scope is active (reuse the picker's own logic)
    let eff_scope: Option<String> = picker.query_scope().or_else(|| picker.scope.clone());
    let show_scope = eff_scope.is_some();
    let scope_height = if show_scope { 1u16 } else { 0u16 };

    let sections = Layout::vertical([
        Constraint::Length(1),            // input
        Constraint::Length(scope_height), // scope line
        Constraint::Min(3),               // list
        Constraint::Length(1),            // status line
    ])
    .split(inner);

    // Input line
    let input_text = format!("> {}", picker.input.value());
    frame.render_widget(Paragraph::new(input_text).style(theme::text()), sections[0]);
    // Show cursor at correct position (offset +2 for "> " prefix)
    let cursor_x = sections[0].x + 2 + picker.input.visual_cursor() as u16;
    let cursor_y = sections[0].y;
    frame.set_cursor_position((cursor_x, cursor_y));

    // Scope line
    if show_scope {
        let eff = eff_scope.unwrap_or_default();
        let scope_line = Line::from(vec![
            Span::styled(format!("  scope: {}/", eff), theme::muted()),
            Span::styled("  CTRL-S: cycle", theme::muted()),
        ]);
        frame.render_widget(Paragraph::new(scope_line), sections[1]);
    }

    // Items list
    let highlight_style = Style::default()
        .fg(theme::MINT)
        .add_modifier(Modifier::BOLD);

    let visible_items: Vec<ListItem> = picker
        .filtered
        .iter()
        .enumerate()
        .map(|(list_idx, &i)| {
            let item = &picker.all_items[i];
            let toggled = picker.toggled.contains(&i);
            let dot = if toggled { "● " } else { "  " };

            let indices = picker
                .match_indices
                .get(list_idx)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let name_spans =
                build_highlighted_spans(&item.name, indices, theme::text(), highlight_style);

            let mut line_spans = vec![Span::styled(dot, Style::default().fg(theme::TEAL))];
            line_spans.extend(name_spans);
            line_spans.push(Span::styled(format!("  ({})", item.parent), theme::muted()));

            ListItem::new(Line::from(line_spans))
        })
        .collect();

    let list = List::new(visible_items).highlight_style(theme::highlight_row());

    let mut list_state = ListState::default();
    if !picker.filtered.is_empty() {
        list_state.select(Some(picker.highlighted));
    }
    frame.render_stateful_widget(list, sections[2], &mut list_state);

    // Status line
    let status = Line::from(vec![
        Span::styled(
            format!("{} selected", picker.toggled.len()),
            Style::default().fg(theme::TEAL),
        ),
        Span::raw("  "),
        Span::styled(
            format!(
                "{}/{} matched",
                picker.filtered.len(),
                picker.all_items.len()
            ),
            theme::muted(),
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
        paths
            .iter()
            .map(|p| PickerItem::from_path(PathBuf::from(p)))
            .collect()
    }

    #[test]
    fn filters_by_query() {
        let items = make_items(&["/work/acme/acme-api", "/work/widgets/widget-ui"]);
        let mut picker = FuzzyPicker::new("test", items, false);
        picker.input = picker.input.with_value("acme".into());
        picker.refilter();
        assert_eq!(picker.filtered.len(), 1);
        assert_eq!(picker.all_items[picker.filtered[0]].name, "acme-api");
    }

    #[test]
    fn scope_filters_by_parent() {
        let items = make_items(&[
            "/work/acme/acme-api",
            "/work/widgets/widget-ui",
            "/work/acme/acme-payments",
        ]);
        let mut picker = FuzzyPicker::new("test", items, false);
        picker.input = picker.input.with_value("acme/".into());
        picker.refilter();
        assert_eq!(picker.filtered.len(), 2);
        assert!(
            picker.all_items[picker.filtered[0]].parent == "acme"
                || picker.all_items[picker.filtered[1]].parent == "acme"
        );
    }

    #[test]
    fn multi_select_toggle() {
        let items = make_items(&["/work/acme/a", "/work/acme/b"]);
        let mut picker = FuzzyPicker::new("test", items, true);
        picker.toggle_highlighted();
        assert_eq!(picker.toggled.len(), 1);
        picker.toggle_highlighted();
        assert_eq!(picker.toggled.len(), 0);
    }
}

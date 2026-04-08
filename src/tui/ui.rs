use crate::tui::app::{App, Pane, Screen};
use crate::tui::theme;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table,
        TableState, Wrap,
    },
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Build a table cell showing "+N -M" with green additions and red deletions.
fn diff_cell(insertions: usize, deletions: usize) -> Cell<'static> {
    if insertions == 0 && deletions == 0 {
        return Cell::from("");
    }
    Cell::from(Line::from(vec![
        Span::styled(format!("+{}", insertions), theme::additions()),
        Span::raw(" "),
        Span::styled(format!("-{}", deletions), theme::deletions()),
    ]))
}

fn status_char(status: &crate::core::git::FileStatus) -> &'static str {
    use crate::core::git::FileStatus;
    match status {
        FileStatus::Modified => "M",
        FileStatus::Added => "A",
        FileStatus::Deleted => "D",
        FileStatus::Renamed => "R",
        FileStatus::Copied => "C",
        FileStatus::Untracked => "?",
    }
}

fn format_repo_status(status: &crate::core::git::RepoStatus, max_width: usize) -> String {
    let mut parts = Vec::new();

    if status.modified > 0 {
        parts.push(format!("{} modified", status.modified));
    }
    if status.staged > 0 {
        parts.push(format!("{} staged", status.staged));
    }
    if status.untracked > 0 {
        parts.push(format!("{} new", status.untracked));
    }

    if parts.is_empty() {
        return "clean".to_string();
    }

    let full = parts.join(", ");
    if line_width(&full) <= max_width {
        return full;
    }

    let changed_count = status.modified + status.staged;
    if changed_count > 0 && status.untracked > 0 {
        let grouped = format!("{} changed, {} new", changed_count, status.untracked);
        if line_width(&grouped) <= max_width {
            return grouped;
        }
    }

    if changed_count > 0 {
        let changed = format!("{} changed", changed_count);
        if line_width(&changed) <= max_width {
            return changed;
        }
    }

    if line_width(&parts[0]) <= max_width {
        return parts[0].clone();
    }

    truncate_for_width(&full, max_width)
}

fn line_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn truncate_for_width(text: &str, max_width: usize) -> String {
    if line_width(text) <= max_width {
        return text.to_string();
    }

    let ellipsis = "...";
    let ellipsis_width = UnicodeWidthStr::width(ellipsis);
    if max_width <= ellipsis_width {
        return ".".repeat(max_width);
    }

    let keep_width = max_width - ellipsis_width;
    let mut width = 0;
    let mut truncated = String::new();

    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > keep_width {
            break;
        }
        truncated.push(ch);
        width += ch_width;
    }

    format!("{}{}", truncated, ellipsis)
}

fn render_delete_footer(
    inner_width: usize,
    footer_delete: &str,
    footer_spacing: &str,
    footer_cancel: &str,
) -> Vec<Line<'static>> {
    let combined_width =
        line_width(footer_delete) + line_width(footer_spacing) + line_width(footer_cancel);
    if combined_width <= inner_width {
        return vec![Line::from(vec![
            Span::styled(footer_delete.to_string(), theme::error()),
            Span::raw(footer_spacing.to_string()),
            Span::styled(footer_cancel.to_string(), theme::muted()),
        ])];
    }

    vec![
        Line::from(Span::styled(
            truncate_for_width(footer_delete, inner_width),
            theme::error(),
        )),
        Line::from(Span::styled(
            truncate_for_width(footer_cancel, inner_width),
            theme::muted(),
        )),
    ]
}

pub fn view(app: &App, frame: &mut Frame) {
    match &app.screen {
        Screen::Dashboard => render_dashboard(app, frame),
        Screen::CreateWorkspace(state) => {
            render_dashboard(app, frame);
            render_create_overlay(state, frame);
        }
        Screen::GoWorkspace(state) => {
            render_dashboard(app, frame);
            crate::tui::widgets::fuzzy_picker::render(&state.picker, frame);
        }
        Screen::AddRepos(state) => {
            render_dashboard(app, frame);
            render_add_overlay(state, frame);
        }
        Screen::ConfirmDelete(state) => {
            render_dashboard(app, frame);
            render_delete_confirm(state, frame);
        }
        Screen::RepoSearch(state) => {
            render_dashboard(app, frame);
            crate::tui::widgets::fuzzy_picker::render(&state.picker, frame);
        }
        Screen::ConfigEditor(state) => render_config_editor(state, frame),
        Screen::Help => {
            render_dashboard(app, frame);
            render_help_overlay(frame);
        }
    }
}

fn render_dashboard(app: &App, frame: &mut Frame) {
    let area = frame.area();

    // Outer layout: title bar / main / status bar
    let outer = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Min(0),    // main
        Constraint::Length(1), // status bar
    ])
    .split(area);

    render_title(frame, outer[0]);
    render_main(app, frame, outer[1]);
    render_status_bar(app, frame, outer[2]);
}

fn render_title(frame: &mut Frame, area: Rect) {
    let title = Line::from(vec![
        Span::styled(" space ", theme::title()),
        Span::styled(format!("v{}", env!("CARGO_PKG_VERSION")), theme::muted()),
    ]);
    frame.render_widget(Paragraph::new(title), area);
}

fn render_main(app: &App, frame: &mut Frame, area: Rect) {
    let panes =
        Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)]).split(area);

    render_workspace_list(app, frame, panes[0]);
    render_repo_table(app, frame, panes[1]);
}

fn render_workspace_list(app: &App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Pane::Left;
    let border_style = if focused {
        theme::border_focused()
    } else {
        theme::border_unfocused()
    };

    let items: Vec<ListItem> = app
        .workspaces
        .iter()
        .map(|ws| {
            let repo_count = ws.repos.len();
            let label = if repo_count > 0 {
                format!("{} ({})", ws.name, repo_count)
            } else {
                ws.name.clone()
            };
            ListItem::new(label)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(" WORKSPACES ");

    if app.workspaces.is_empty() {
        let empty_msg = Paragraph::new("No workspaces yet\n\nPress c to create one")
            .style(theme::muted())
            .alignment(Alignment::Center)
            .block(block);
        frame.render_widget(empty_msg, area);
        return;
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected())
        .highlight_symbol("> ");

    let mut state = ListState::default();
    state.select(Some(app.selected_ws));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_repo_table(app: &App, frame: &mut Frame, area: Rect) {
    use crate::tui::app::RepoRow;

    let focused = app.focus == Pane::Right;
    let border_style = if focused {
        theme::border_focused()
    } else {
        theme::border_unfocused()
    };

    let ws_name = app
        .selected_workspace()
        .map(|ws| ws.name.as_str())
        .unwrap_or("");
    let target_label = match app.diff_target {
        crate::core::git::DiffTarget::Head => "HEAD",
        crate::core::git::DiffTarget::Base => "base",
    };
    let title = format!(" {} (vs {}) ", ws_name, target_label);
    let status_width = area.width.saturating_sub(2) as usize * 36 / 100;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(title);

    let rows_data = app.flattened_rows();

    if rows_data.is_empty() {
        frame.render_widget(Paragraph::new("  No repos").block(block), area);
        return;
    }

    let rows: Vec<Row> = rows_data
        .iter()
        .map(|row| match row {
            RepoRow::Repo {
                index,
                repo: r,
                expanded,
            } => {
                let indicator = if *expanded { "▼ " } else { "▶ " };
                let name = format!("{}{}", indicator, r.name);
                let dirty = r.status.modified + r.status.staged + r.status.untracked > 0;
                let status_style = if dirty {
                    theme::warn()
                } else {
                    theme::status_clean()
                };
                let status_str = format_repo_status(&r.status, status_width);
                // +/- column: file line totals from cache. Show zeros when the
                // cache isn't populated yet rather than falling back to commit
                // counts (ahead/behind), which would mix different metrics.
                let (ins, del) = app
                    .repo_file_cache
                    .get(index)
                    .map(|entries| {
                        entries.iter().fold((0usize, 0usize), |(i, d), e| {
                            (i + e.insertions, d + e.deletions)
                        })
                    })
                    .unwrap_or((0, 0));
                Row::new(vec![
                    Cell::from(Span::raw(name)),
                    Cell::from(Span::styled(r.branch.clone(), theme::branch())),
                    Cell::from(Span::styled(status_str, status_style)),
                    diff_cell(ins, del),
                ])
            }
            RepoRow::File { entry, .. } => {
                // In Base mode all entries have staged=false (committed divergence has
                // no staging context), so the badge would always show "[unstaged]" which
                // is misleading. Hide it entirely in Base mode.
                let staged_badge = match app.diff_target {
                    crate::core::git::DiffTarget::Head if entry.staged => {
                        ratatui::text::Span::styled("[staged]", theme::staged())
                    }
                    crate::core::git::DiffTarget::Head => {
                        ratatui::text::Span::styled("[unstaged]", theme::unstaged())
                    }
                    crate::core::git::DiffTarget::Base => ratatui::text::Span::raw(""),
                };
                let path_col = format!("  {} {}", status_char(&entry.status), entry.path);
                Row::new(vec![
                    Cell::from(Span::styled(path_col, theme::file_path())),
                    Cell::from(""),
                    Cell::from(staged_badge), // col 3 = STATUS header
                    diff_cell(entry.insertions, entry.deletions), // col 4 = +/- header
                ])
            }
        })
        .collect();

    let header = Row::new(vec!["REPO", "BRANCH", "STATUS", "+/-"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(32),
            Constraint::Percentage(22),
            Constraint::Percentage(36),
            Constraint::Percentage(10),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(theme::highlight_row());

    let mut state = TableState::default();
    if !rows_data.is_empty() && focused {
        state.select(Some(app.cursor_row));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    if let Some(msg) = &app.status_message {
        frame.render_widget(Paragraph::new(msg.as_str()).style(theme::muted()), area);
        return;
    }

    let bindings = crate::tui::keybindings::status_bar_bindings(app.focus);
    let sep = Span::styled("  ·  ", theme::muted());
    let mut spans: Vec<Span> = Vec::new();

    // Dynamic T-label for right pane: tell user which direction T will go
    let t_override: Option<&'static str> = if app.focus == crate::tui::app::Pane::Right {
        Some(match app.diff_target {
            crate::core::git::DiffTarget::Base => "switch to HEAD",
            crate::core::git::DiffTarget::Head => "switch to base",
        })
    } else {
        None
    };

    for (i, binding) in bindings.iter().enumerate() {
        if i > 0 {
            spans.push(sep.clone());
        }
        spans.push(Span::styled(binding.key, theme::text()));
        // Use dynamic label for T on right pane, static label otherwise
        let desc = if binding.key == "T" {
            t_override.unwrap_or(binding.desc)
        } else {
            binding.desc
        };
        spans.push(Span::styled(format!(" {}", desc), theme::muted()));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_create_overlay(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    use crate::tui::screens::create::CreateStage;
    match &state.stage {
        CreateStage::PickRepos => {
            crate::tui::widgets::fuzzy_picker::render(&state.picker, frame);
        }
        CreateStage::NameWorkspace => render_name_input(state, frame),
        CreateStage::PickBranchStrategy => render_branch_strategy_picker(
            frame,
            state.ws_name.value(),
            state.branch_strategy_idx,
            state.error.as_deref(),
            &state.recent_branches,
        ),
        CreateStage::PickBranch => {
            if let Some(ref picker) = state.branch_picker {
                crate::tui::widgets::fuzzy_picker::render(picker, frame);
            }
        }
        CreateStage::Creating => render_worktree_progress(
            frame,
            " Creating Workspace ",
            &state.progress,
            state.error.as_deref(),
        ),
    }
}

fn render_name_input(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let area = centered_rect_fixed(50, 7, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(" Workspace Name ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new("Enter workspace name:").style(theme::text()),
        sections[0],
    );
    frame.render_widget(
        Paragraph::new(format!("> {}", state.ws_name.value())).style(theme::input_style()),
        sections[1],
    );
    if let Some(err) = &state.error {
        frame.render_widget(
            Paragraph::new(err.as_str()).style(theme::error()),
            sections[2],
        );
    }
}

fn render_branch_strategy_picker(
    frame: &mut Frame,
    workspace_name: &str,
    strategy_idx: usize,
    error: Option<&str>,
    recent_branches: &[crate::core::git::BranchInfo],
) {
    use ratatui::widgets::Clear;
    let has_error = error.is_some();
    let n = recent_branches.len();
    let branch_rows = if n > 0 { 1 + n as u16 + 1 } else { 1 };
    let content_rows = 3 + branch_rows;
    let height: u16 = content_rows + 2 + if has_error { 3 } else { 1 };
    let area = centered_rect_fixed(62, height, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if has_error {
        theme::border_danger()
    } else {
        theme::border_focused()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(" Branch Strategy ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = if has_error {
        Layout::vertical([
            Constraint::Length(content_rows),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .split(inner)
    } else {
        Layout::vertical([Constraint::Length(content_rows), Constraint::Min(0)]).split(inner)
    };

    let mut items: Vec<ListItem> = Vec::new();

    // Fixed options (selectable indices 0, 1, 2)
    let fixed = [
        format!("New branch '{}'", workspace_name),
        format!("Existing branch '{}' (if present)", workspace_name),
        "Detached HEAD".to_string(),
    ];
    for (i, opt) in fixed.iter().enumerate() {
        if i == strategy_idx {
            items.push(ListItem::new(format!("> {}", opt)).style(theme::selected()));
        } else {
            items.push(ListItem::new(format!("  {}", opt)));
        }
    }

    if n > 0 {
        // "Pick a branch..." header (non-selectable, dimmed)
        items.push(ListItem::new("  Pick a branch...").style(theme::muted()));

        // Recent branches (selectable indices 3..3+n)
        for (i, branch) in recent_branches.iter().enumerate() {
            let sel_idx = 3 + i;
            let time_str = crate::core::git::relative_time(branch.last_commit_time);
            let max_name = 56_usize.saturating_sub(time_str.len() + 2);
            let display_name = if branch.name.len() > max_name {
                let truncated: String = branch
                    .name
                    .chars()
                    .take(max_name.saturating_sub(3))
                    .collect();
                format!("{}...", truncated)
            } else {
                branch.name.clone()
            };
            let padding = 56_usize.saturating_sub(display_name.len() + time_str.len());
            let line = format!("{}{}{}", display_name, " ".repeat(padding), time_str);
            if sel_idx == strategy_idx {
                items.push(ListItem::new(format!("  > {}", line)).style(theme::selected()));
            } else {
                items.push(ListItem::new(format!("    {}", line)));
            }
        }

        // "Show more..." (selectable index 3+n)
        let show_more_idx = 3 + n;
        if show_more_idx == strategy_idx {
            items.push(ListItem::new("  > Show more...").style(theme::selected()));
        } else {
            items.push(ListItem::new("    Show more..."));
        }
    } else {
        // No recent branches — "Pick a branch..." as selectable (idx 3)
        if 3 == strategy_idx {
            items.push(ListItem::new("> Pick a branch...").style(theme::selected()));
        } else {
            items.push(ListItem::new("  Pick a branch..."));
        }
    }

    frame.render_widget(List::new(items), sections[0]);

    if let Some(err) = error {
        frame.render_widget(
            Paragraph::new(format!("\u{26a0}  {}", err))
                .style(theme::error())
                .wrap(Wrap { trim: false }),
            sections[2],
        );
    }
}

fn render_worktree_progress(
    frame: &mut Frame,
    title: &str,
    progress: &[String],
    error: Option<&str>,
) {
    use ratatui::widgets::Clear;
    let area = centered_rect_fixed(60, 15, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = progress
        .iter()
        .map(|l| {
            if l.starts_with("  \u{2713}") {
                Line::from(Span::styled(l.clone(), theme::success()))
            } else if l.starts_with("  \u{2717}") {
                Line::from(Span::styled(l.clone(), theme::error()))
            } else {
                Line::from(Span::raw(l.clone()))
            }
        })
        .collect();

    let sections = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(inner);

    frame.render_widget(Paragraph::new(lines), sections[0]);

    if let Some(err) = error {
        frame.render_widget(
            Paragraph::new(format!("Error: {}  [ESC to dismiss]", err)).style(theme::error()),
            sections[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new("Done! [ENTER to continue]").style(theme::success()),
            sections[1],
        );
    }
}

fn render_add_overlay(state: &crate::tui::screens::add::AddState, frame: &mut Frame) {
    use crate::tui::screens::add::AddStage;
    match &state.stage {
        AddStage::PickRepos => {
            crate::tui::widgets::fuzzy_picker::render(&state.picker, frame);
        }
        AddStage::PickBranchStrategy => render_branch_strategy_picker(
            frame,
            &state.workspace_name,
            state.branch_strategy_idx,
            state.error.as_deref(),
            &state.recent_branches,
        ),
        AddStage::PickBranch => {
            if let Some(ref picker) = state.branch_picker {
                crate::tui::widgets::fuzzy_picker::render(picker, frame);
            }
        }
        AddStage::Creating => render_worktree_progress(
            frame,
            " Adding Repos ",
            &state.progress,
            state.error.as_deref(),
        ),
    }
}

fn render_delete_confirm(state: &crate::tui::screens::delete::DeleteState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let title = " Delete Workspace ";
    let heading = "Delete workspace?";
    let repo_heading = "This removes these worktrees:";
    let footer_delete = "Enter/y delete";
    let footer_cancel = "Esc/n cancel";
    let footer_spacing = "   ";
    let footer_text = format!("{}{}{}", footer_delete, footer_spacing, footer_cancel);

    let max_outer_width = frame.area().width.saturating_sub(4).max(20);
    let max_inner_width = max_outer_width.saturating_sub(2) as usize;
    let desired_inner_width = [
        line_width(heading),
        line_width(&state.workspace_name),
        line_width(repo_heading),
        line_width(&footer_text),
        state
            .repo_names
            .iter()
            .map(|name| line_width(name) + 2)
            .max()
            .unwrap_or(0),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let inner_width = desired_inner_width
        .min(max_inner_width)
        .max(18)
        .min(frame.area().width.saturating_sub(2) as usize);
    let footer_line_count =
        render_delete_footer(inner_width, footer_delete, footer_spacing, footer_cancel).len();

    let max_outer_height = frame.area().height.saturating_sub(2).max(8);
    let max_inner_height = max_outer_height.saturating_sub(2) as usize;
    let static_line_count = 5usize + footer_line_count;
    let available_repo_lines = max_inner_height.saturating_sub(static_line_count);

    let mut visible_repo_names = Vec::new();
    let mut overflow_count = 0usize;
    if state.repo_names.len() <= available_repo_lines {
        visible_repo_names.extend(state.repo_names.iter().cloned());
    } else if available_repo_lines > 0 {
        let visible_count = available_repo_lines.saturating_sub(1);
        visible_repo_names.extend(state.repo_names.iter().take(visible_count).cloned());
        overflow_count = state.repo_names.len().saturating_sub(visible_count);
    } else {
        overflow_count = state.repo_names.len();
    }

    let content_line_count =
        static_line_count + visible_repo_names.len() + usize::from(overflow_count > 0);
    let outer_width = (inner_width as u16)
        .saturating_add(2)
        .min(frame.area().width);
    let outer_height = (content_line_count as u16 + 2).min(frame.area().height);
    let area = centered_rect_fixed(outer_width, outer_height, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_danger())
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            truncate_for_width(heading, inner.width as usize),
            theme::text().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            truncate_for_width(&state.workspace_name, inner.width as usize),
            theme::text(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            truncate_for_width(repo_heading, inner.width as usize),
            theme::muted(),
        )),
    ];

    for name in &visible_repo_names {
        lines.push(Line::from(Span::styled(
            truncate_for_width(&format!("  {}", name), inner.width as usize),
            theme::dim_text(),
        )));
    }

    if overflow_count > 0 {
        lines.push(Line::from(Span::styled(
            truncate_for_width(
                &format!("  ... and {} more", overflow_count),
                inner.width as usize,
            ),
            theme::muted(),
        )));
    }

    lines.push(Line::from(""));
    lines.extend(render_delete_footer(
        inner.width as usize,
        footer_delete,
        footer_spacing,
        footer_cancel,
    ));

    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_for_width_uses_display_width() {
        assert_eq!(line_width("界界"), 4);
        assert_eq!(truncate_for_width("界界abc", 5), "界...");
    }

    #[test]
    fn delete_footer_wraps_when_it_cannot_fit_on_one_line() {
        let lines = render_delete_footer(18, "Enter/y delete", "   ", "Esc/n cancel");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].width(), 14);
        assert_eq!(lines[1].width(), 12);
    }
}

fn render_config_editor(state: &crate::tui::screens::config::ConfigState, frame: &mut Frame) {
    use ratatui::widgets::Clear;

    let area = frame.area();
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(" Configuration ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Layout: per field → label row (1) + value row (1) + gap (1) = 3 rows each
    // Plus a spacer + hint bar at bottom
    let mut constraints: Vec<Constraint> = state
        .fields
        .iter()
        .flat_map(|_| {
            [
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ]
        })
        .collect();
    constraints.push(Constraint::Min(0)); // spacer
    constraints.push(Constraint::Length(1)); // hint bar
    let sections = Layout::vertical(constraints).split(inner);

    for (i, field) in state.fields.iter().enumerate() {
        let label_area = sections[i * 3];
        let value_area = sections[i * 3 + 1];
        // sections[i * 3 + 2] is the gap row — intentionally empty

        let is_focused = i == state.focused;

        // Label row: "Label  hint"
        let label_line = if field.hint.is_empty() {
            ratatui::text::Line::from(ratatui::text::Span::styled(
                field.label,
                if is_focused {
                    theme::selected()
                } else {
                    theme::text()
                },
            ))
        } else {
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(
                    field.label,
                    if is_focused {
                        theme::selected()
                    } else {
                        theme::text()
                    },
                ),
                ratatui::text::Span::raw("  "),
                ratatui::text::Span::styled(field.hint, theme::muted()),
            ])
        };
        frame.render_widget(Paragraph::new(label_line), label_area);

        // Value row
        if is_focused && state.editing {
            // Show input value with blinking cursor
            frame.render_widget(
                Paragraph::new(state.input.value()).style(theme::input_style()),
                value_area,
            );
            // Set terminal cursor position
            let cursor_x = value_area.x + state.input.visual_cursor() as u16;
            let cursor_y = value_area.y;
            frame.set_cursor_position((cursor_x, cursor_y));
        } else {
            let value_style = if is_focused {
                theme::border_focused() // TEAL for focused-not-editing
            } else {
                theme::dim_text()
            };
            frame.render_widget(
                Paragraph::new(field.value.clone()).style(value_style),
                value_area,
            );
        }
    }

    // Hint bar
    let hint_idx = state.fields.len() * 3 + 1;
    frame.render_widget(
        Paragraph::new("↑↓ navigate  ·  Enter edit  ·  Esc cancel  ·  Ctrl-S save")
            .style(theme::muted()),
        sections[hint_idx],
    );
}

fn render_help_overlay(frame: &mut Frame) {
    use ratatui::widgets::Clear;

    let groups = crate::tui::keybindings::all_groups();

    // Calculate height: 1 header + N bindings per group + 1 gap between groups + 1 bottom hint
    let content_rows: u16 = groups
        .iter()
        .map(|g| 1 + g.bindings.len() as u16)
        .sum::<u16>()
        + (groups.len() as u16).saturating_sub(1) // gaps between groups
        + 1; // bottom hint line
    let height = (content_rows + 2).min(frame.area().height); // +2 for border
                                                              // Height is clamped to terminal height — content clips on very short terminals
                                                              // (< 27 rows). Acceptable: 24+ rows is the practical minimum for a terminal.
    let area = centered_rect_fixed(50, height, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(" Help ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    for (i, group) in groups.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from("")); // gap between groups
        }
        lines.push(Line::from(Span::styled(group.name, theme::title())));
        for binding in group.bindings {
            let padding = 12_usize.saturating_sub(binding.key.chars().count());
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {}{}", binding.key, " ".repeat(padding)),
                    theme::text().add_modifier(Modifier::BOLD),
                ),
                Span::styled(binding.desc, theme::muted()),
            ]));
        }
    }

    let sections = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

    frame.render_widget(Paragraph::new(lines), sections[0]);
    frame.render_widget(
        Paragraph::new("Esc / q / ? to close")
            .style(theme::muted())
            .alignment(Alignment::Center),
        sections[1],
    );
}

fn centered_rect_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

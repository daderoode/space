use crate::tui::app::{App, Pane, Screen};
use crate::tui::screens::sync_report::{self, LogView, SyncReport};
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
        FileStatus::Conflicted => "!",
    }
}

/// Minimum virtual content widths (display chars) for the scrollable columns.
/// REPO is pinned. BRANCH and STATUS form the scrollable zone.
const BRANCH_VIRTUAL: usize = 50;
const STATUS_VIRTUAL: usize = 45;

/// Compute the maximum horizontal table scroll for the given right-pane inner
/// width (pane width minus 2 for borders). Returns 0 when the terminal is wide
/// enough that all content fits without scrolling.
/// Inner width = right pane width - 2.
pub(crate) fn max_table_scroll(inner_width: usize) -> u16 {
    let branch_display = ((inner_width as f64 * 22.0) / 100.0).round() as usize;
    let status_display = ((inner_width as f64 * 36.0) / 100.0).round() as usize;
    (BRANCH_VIRTUAL + STATUS_VIRTUAL).saturating_sub(branch_display + status_display) as u16
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
    if status.conflicted > 0 {
        parts.push(format!(
            "{} {}",
            status.conflicted,
            if status.conflicted == 1 {
                "conflict"
            } else {
                "conflicts"
            }
        ));
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

/// Skip `skip` display-width characters from the start of `s`.
/// Returns the remaining substring starting from the first character
/// whose cumulative display width reaches or exceeds `skip`.
/// Returns `""` when `skip` >= total display width of `s`.
/// When `skip` bisects a wide character (display width > 1), the entire
/// wide character is skipped (snap-forward policy).
fn skip_display_width(s: &str, skip: usize) -> &str {
    if skip == 0 {
        return s;
    }
    let mut consumed = 0usize;
    for (byte_idx, ch) in s.char_indices() {
        if consumed >= skip {
            return &s[byte_idx..];
        }
        consumed += UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    ""
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
        Screen::FilterWorkspace(state) => {
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
        Screen::DiffViewer(state) => {
            render_dashboard(app, frame);
            render_diff_overlay(state, frame);
        }
        Screen::Help => {
            render_dashboard(app, frame);
            render_help_overlay(frame);
        }
        Screen::SwitchBranch(state) => {
            render_dashboard(app, frame);
            render_switch_branch_overlay(state, frame);
        }
        Screen::GitOps(state) => {
            render_dashboard(app, frame);
            render_gitops_overlay(state, app.spinner_tick, frame);
        }
    }
}

fn render_dashboard(app: &App, frame: &mut Frame) {
    let area = frame.area();

    // Guard: the dashboard layout requires at least 80 columns for the repo
    // table columns and dialogs to remain readable. Overlay screens (delete
    // confirm, etc.) render on top of the dashboard and handle narrow widths
    // themselves, so this guard only affects the background dashboard layer.
    const MIN_DASHBOARD_WIDTH: u16 = 80;
    if area.width < MIN_DASHBOARD_WIDTH {
        let msg = format!(
            "Terminal too narrow\nMinimum: {} columns | Current: {}",
            MIN_DASHBOARD_WIDTH, area.width,
        );
        frame.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(theme::error()),
            centered_rect_percent(80, 30, area),
        );
        return;
    }

    // Outer layout: title bar / main / (collapsible) status message / keybindings bar
    let status_height: u16 = if app.status_message.is_some() { 1 } else { 0 };
    let outer = Layout::vertical([
        Constraint::Length(1),             // title
        Constraint::Min(0),                // main
        Constraint::Length(status_height), // status message (collapses when idle)
        Constraint::Length(1),             // keybindings (always visible)
    ])
    .split(area);

    render_title(frame, outer[0]);
    render_main(app, frame, outer[1]);
    if app.status_message.is_some() {
        render_status_message(app, frame, outer[2]);
    }
    render_keybindings_bar(app, frame, outer[3]);
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
    // Horizontal scroll: REPO is pinned, BRANCH+STATUS are the scrollable zone.
    let inner_width = area.width.saturating_sub(2) as usize;
    // Match ratatui's Percentage layout rounding (nearest integer, not floor) so that
    // max_scroll reaches exactly 0 at the same terminal width where the columns become
    // wide enough to display all content without clipping.
    let branch_display = ((inner_width as f64 * 22.0) / 100.0).round() as usize;
    let status_display = ((inner_width as f64 * 36.0) / 100.0).round() as usize; // same as status_width
    let scrollable_virtual = BRANCH_VIRTUAL + STATUS_VIRTUAL; // 95
    let scrollable_display = branch_display + status_display;
    let max_scroll = scrollable_virtual.saturating_sub(scrollable_display);
    let scroll_x = (app.table_scroll_x as usize).min(max_scroll);
    let branch_offset = scroll_x;
    let status_offset = scroll_x.saturating_sub(BRANCH_VIRTUAL);

    let left_ind = if scroll_x > 0 { " <" } else { "" };
    let right_ind = if scroll_x < max_scroll { " >" } else { "" };
    let title = if app.ws_loading
        && app
            .ws_loading_since
            .map(|t| t.elapsed() > std::time::Duration::from_millis(200))
            .unwrap_or(false)
    {
        let frames = ["·  ", "·· ", "···"];
        let frame = frames[(app.spinner_tick as usize / 30) % 3];
        format!(" {}{}{} {} ", ws_name, left_ind, right_ind, frame)
    } else {
        format!(" {}{}{} ", ws_name, left_ind, right_ind)
    };

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
                let dirty =
                    r.status.modified + r.status.staged + r.status.untracked + r.status.conflicted
                        > 0;
                let status_style = if dirty {
                    theme::warn()
                } else {
                    theme::status_clean()
                };
                let status_str = format_repo_status(&r.status, status_display + status_offset);
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
                let branch_cell = if app.ws_loading && r.branch == "..." {
                    Cell::from("...").style(theme::muted())
                } else {
                    let b = skip_display_width(&r.branch, branch_offset);
                    let b = truncate_for_width(b, branch_display);
                    Cell::from(Span::styled(b, theme::branch()))
                };
                Row::new(vec![
                    Cell::from(Span::raw(name)),
                    branch_cell,
                    {
                        let s = skip_display_width(&status_str, status_offset);
                        let s = truncate_for_width(s, status_display);
                        Cell::from(Span::styled(s, status_style))
                    },
                    diff_cell(ins, del),
                ])
            }
            RepoRow::SectionHeader { label, .. } => {
                let label_text = format!("  ── {} ──", label);
                Row::new(vec![
                    Cell::from(Span::styled(label_text, theme::muted())),
                    Cell::from(""),
                    Cell::from(""),
                    Cell::from(""),
                ])
            }
            RepoRow::File {
                entry,
                partially_staged,
                ..
            } => {
                let is_conflicted = entry.status == crate::core::git::FileStatus::Conflicted;
                let badge = if is_conflicted {
                    ratatui::text::Span::styled("[conflict]", theme::error())
                } else if *partially_staged {
                    ratatui::text::Span::styled("[partial]", theme::warn())
                } else {
                    ratatui::text::Span::raw("")
                };
                let path_col = format!("  {} {}", status_char(&entry.status), entry.path);
                let path_style = if is_conflicted {
                    theme::error()
                } else {
                    theme::file_path()
                };
                Row::new(vec![
                    Cell::from(Span::styled(path_col, path_style)),
                    Cell::from(""),
                    Cell::from(badge),
                    diff_cell(entry.insertions, entry.deletions),
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

/// Render the colored status message row (only called when a message is set).
fn render_status_message(app: &App, frame: &mut Frame, area: Rect) {
    use crate::tui::actions::StatusKind;
    if let Some(msg) = &app.status_message {
        let style = match app.status_kind {
            StatusKind::Error => theme::error(),
            StatusKind::Success => theme::success(),
            StatusKind::Warning => theme::warn(),
            StatusKind::Info => theme::muted(),
        };
        frame.render_widget(Paragraph::new(msg.as_str()).style(style), area);
    }
}

/// Render the always-visible keybindings hint bar at the bottom.
fn render_keybindings_bar(app: &App, frame: &mut Frame, area: Rect) {
    let bindings = crate::tui::keybindings::status_bar_bindings(app.focus);
    let sep = Span::styled("  ·  ", theme::muted());
    let mut spans: Vec<Span> = Vec::new();

    for (i, binding) in bindings.iter().enumerate() {
        if i > 0 {
            spans.push(sep.clone());
        }
        spans.push(Span::styled(binding.key, theme::text()));
        spans.push(Span::styled(format!(" {}", binding.desc), theme::muted()));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_create_overlay(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    use crate::tui::screens::create::CreateStage;
    match &state.stage {
        CreateStage::EnterName => render_name_input(state, frame),
        CreateStage::PickRepos => {
            crate::tui::widgets::fuzzy_picker::render(&state.picker, frame);
        }
        CreateStage::PickBranchStrategy => render_branch_strategy_picker(
            frame,
            state.ws_name.value(),
            state.branch_strategy_idx,
            state.error.as_deref(),
            &state.recent_branches,
        ),
        CreateStage::EnterBranchName => render_text_input_dialog(
            "Branch Name",
            "New branch name:",
            &state.branch_name_input,
            state.error.as_deref(),
            frame,
        ),
        CreateStage::PickBranch => {
            if let Some(ref picker) = state.branch_picker {
                crate::tui::widgets::fuzzy_picker::render(picker, frame);
            }
        }
        CreateStage::Syncing => render_sync_report(frame, &state.report),
        CreateStage::Creating => render_creating_log(
            frame,
            " Creating Workspace ",
            &state.progress,
            state.error.as_deref(),
            &state.log_view,
        ),
    }
}

fn render_text_input_dialog(
    title: &str,
    prompt: &str,
    input: &tui_input::Input,
    error: Option<&str>,
    frame: &mut Frame,
) {
    use ratatui::widgets::Clear;
    let dialog_w = (frame.area().width * 70 / 100).max(50);
    let area = centered_rect_fixed(dialog_w, 7, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(format!(" {} ", title));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    frame.render_widget(Paragraph::new(prompt).style(theme::text()), sections[0]);

    // Split the input row into: left-scroll-indicator | text area | right-scroll-indicator
    let indicator_style = theme::muted();
    let text_area_w = sections[1].width.saturating_sub(2); // 1 char each side for indicators
    let text_area = Rect {
        x: sections[1].x + 1,
        y: sections[1].y,
        width: text_area_w,
        height: 1,
    };
    let left_ind_area = Rect {
        x: sections[1].x,
        y: sections[1].y,
        width: 1,
        height: 1,
    };
    let right_ind_area = Rect {
        x: sections[1].x + 1 + text_area_w,
        y: sections[1].y,
        width: 1,
        height: 1,
    };

    // Compute horizontal scroll to keep cursor in the visible text area
    let scroll = input.visual_scroll(text_area_w as usize) as u16;
    let cursor_col = input.visual_cursor() as u16;
    let value_vis_w = UnicodeWidthStr::width(input.value()) as u16;

    // Left indicator: ‹ when text is scrolled (content hidden on left)
    let left_text = if scroll > 0 { "\u{2039}" } else { " " }; // ‹
    frame.render_widget(
        Paragraph::new(left_text).style(indicator_style),
        left_ind_area,
    );

    // Text with horizontal scroll
    frame.render_widget(
        Paragraph::new(input.value())
            .style(theme::input_style())
            .scroll((0, scroll)),
        text_area,
    );

    // Right indicator: › when content extends beyond the visible right edge.
    let right_text = if value_vis_w > text_area_w + scroll {
        "\u{203a}" // ›
    } else {
        " "
    };
    frame.render_widget(
        Paragraph::new(right_text).style(indicator_style),
        right_ind_area,
    );

    // Cursor: position within the visible text area (clamped to bounds)
    let cursor_x = (text_area.x + cursor_col.saturating_sub(scroll))
        .min(text_area.x + text_area_w.saturating_sub(1));
    frame.set_cursor_position((cursor_x, text_area.y));

    if let Some(err) = error {
        frame.render_widget(Paragraph::new(err).style(theme::error()), sections[2]);
    }
}

fn render_name_input(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    render_text_input_dialog(
        "Workspace Name",
        "Enter workspace name:",
        &state.ws_name,
        state.error.as_deref(),
        frame,
    );
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
    let dialog_w = (frame.area().width * 70 / 100).max(62);
    let area = centered_rect_fixed(dialog_w, height, frame.area());
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

    // Fixed options (selectable indices 0, 1, 2).
    // Truncate to fit the dialog inner width (minus 4 for the "> " or "  " prefix).
    let opt_max_w = dialog_w.saturating_sub(2 + 4) as usize;
    let fixed = [
        truncate_for_width(&format!("New branch '{}'", workspace_name), opt_max_w),
        truncate_for_width(
            &format!("Existing branch '{}' (if present)", workspace_name),
            opt_max_w,
        ),
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
            let inner_w = dialog_w.saturating_sub(2) as usize;
            // Use display widths (not byte lengths) so non-ASCII branch names align correctly.
            let time_w = line_width(&time_str);
            let max_name = inner_w.saturating_sub(6).saturating_sub(time_w + 2);
            let display_name = truncate_for_width(&branch.name, max_name);
            let display_w = line_width(&display_name);
            let content_w = inner_w.saturating_sub(6);
            let padding = content_w.saturating_sub(display_w + time_w);
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

/// Shared dialog geometry for the Syncing and Creating stages: 70% of the
/// frame wide (minimum 60), as tall as `content_rows` plus chrome (borders
/// and footer) up to 80% of the frame height, never under 10 rows. Below the
/// cap the dialog is exactly as tall as its content; at the cap it scrolls.
fn progress_dialog_area(frame_area: Rect, content_rows: usize) -> Rect {
    let dialog_w = progress_dialog_width(frame_area);
    let cap = (usize::from(frame_area.height) * 80 / 100).max(10);
    let height = (content_rows + 3).clamp(10, cap);
    centered_rect_fixed(dialog_w, height as u16, frame_area)
}

/// 70% of the frame, minimum 60, never wider than the frame. Computed in
/// `usize` so a very wide frame cannot overflow `u16`.
fn progress_dialog_width(frame_area: Rect) -> u16 {
    let w = usize::from(frame_area.width);
    (w * 70 / 100).max(60).min(w) as u16
}

/// Pad `text` with spaces to `width` display columns.
fn pad_to(text: &str, width: usize) -> String {
    let w = line_width(text);
    if w >= width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - w))
    }
}

/// The sync report: rows in selection order, a rule, the detail pane for the
/// highlighted row, and the footer. See `screens::sync_report` for the text.
fn render_sync_report(frame: &mut Frame, report: &SyncReport) {
    use ratatui::widgets::Clear;
    let frame_area = frame.area();
    let dialog_w = progress_dialog_width(frame_area);
    let inner_w = usize::from(dialog_w.saturating_sub(2));
    let pane = report.pane(inner_w);
    let n = report.rows.len();
    // List (at least its minimum rows), blank separator, rule, pane.
    let list_need = n.max(sync_report::MIN_LIST_ROWS);
    let area = progress_dialog_area(frame_area, list_need + 2 + pane.lines.len());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(format!(" {} ", report.title()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (list_rows, pane_rows) =
        sync_report::report_layout(usize::from(inner.height), n, pane.lines.len());
    let sections = Layout::vertical([
        Constraint::Length(list_rows as u16),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(pane_rows as u16),
        Constraint::Length(1),
    ])
    .split(inner);

    let (start, end) = report.visible_window(list_rows);
    let repo_w = report
        .rows
        .iter()
        .map(|r| line_width(&r.name))
        .max()
        .unwrap_or(0)
        .min(inner_w / 3);
    let rows: Vec<Line> = (start..end)
        .map(|i| sync_report_row(&report.rows[i], i == report.cursor, repo_w, inner_w))
        .collect();
    frame.render_widget(Paragraph::new(rows), sections[0]);

    frame.render_widget(
        Paragraph::new("\u{2500}".repeat(inner_w)).style(theme::muted()),
        sections[2],
    );

    let pane_lines: Vec<Line> = sync_report::fit_pane(&pane, pane_rows)
        .into_iter()
        .map(|l| {
            if l.dim {
                Line::from(Span::styled(l.text, theme::muted()))
            } else {
                Line::from(l.text)
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(pane_lines), sections[3]);

    // A frame too short for the dialog leaves the list with zero rows and an
    // empty window; that is not a scrolled list, so no `rows a–b of n`.
    let scrolled = (list_rows > 0 && n > list_rows).then_some((start + 1, end));
    frame.render_widget(
        Paragraph::new(report.footer(scrolled, inner_w)).style(theme::muted()),
        sections[4],
    );
}

/// One report row: cursor marker, glyph, REPO, OUTCOME, DETAIL. The REPO
/// column is `repo_w` wide; DETAIL takes what is left and is truncated.
fn sync_report_row(
    row: &sync_report::SyncRow,
    is_cursor: bool,
    repo_w: usize,
    inner_w: usize,
) -> Line<'static> {
    use sync_report::RowPhase;
    const OUTCOME_W: usize = 14; // "fast-forwarded"
    let glyph_style = match &row.phase {
        RowPhase::Waiting => theme::muted(),
        RowPhase::Syncing => theme::warn(),
        RowPhase::Done(o) if o.fetch_ok() => theme::success(),
        RowPhase::Done(_) | RowPhase::NotSynced => theme::error(),
    };
    let marker = if is_cursor { "\u{25b6} " } else { "  " };
    let name = pad_to(&sync_report::truncate_to(&row.name, repo_w), repo_w);
    let outcome = pad_to(row.outcome_label(), OUTCOME_W);
    let detail_w = inner_w.saturating_sub(2 + 1 + 1 + repo_w + 2 + OUTCOME_W + 2);
    let (plain, dim) = row.detail();
    let plain = sync_report::truncate_to(&plain, detail_w);
    let dim = sync_report::truncate_to(&dim, detail_w.saturating_sub(line_width(&plain)));
    let mut line = Line::from(vec![
        Span::raw(marker),
        Span::styled(row.glyph().to_string(), glyph_style),
        Span::raw(" "),
        Span::raw(name),
        Span::raw("  "),
        Span::raw(outcome),
        Span::raw("  "),
        Span::raw(plain),
        Span::styled(dim, theme::muted()),
    ]);
    if is_cursor {
        line = line.style(theme::highlight_row());
    }
    line
}

/// The Creating stage's string log. Grows with its content up to the cap,
/// then scrolls: tail-following while lines arrive, detached by the cursor
/// keys, resumed by `End` (see `LogView`).
fn render_creating_log(
    frame: &mut Frame,
    title: &str,
    progress: &[String],
    error: Option<&str>,
    log: &LogView,
) {
    use ratatui::widgets::Clear;
    let area = progress_dialog_area(frame.area(), progress.len());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    let visible = usize::from(sections[0].height);
    let offset = log.offset(progress.len(), visible);
    let lines: Vec<Line> = progress
        .iter()
        .skip(offset)
        .take(visible)
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
        AddStage::EnterBranchName => render_text_input_dialog(
            "Branch Name",
            "New branch name:",
            &state.branch_name_input,
            state.error.as_deref(),
            frame,
        ),
        AddStage::PickBranch => {
            if let Some(ref picker) = state.branch_picker {
                crate::tui::widgets::fuzzy_picker::render(picker, frame);
            }
        }
        AddStage::Syncing => render_sync_report(frame, &state.report),
        AddStage::Creating => render_creating_log(
            frame,
            " Adding Repos ",
            &state.progress,
            state.error.as_deref(),
            &state.log_view,
        ),
    }
}

fn render_delete_confirm(state: &crate::tui::screens::delete::DeleteState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let title = " Delete Workspace ";
    let heading = "Delete workspace?";
    let repo_heading = "This removes these worktrees:";
    let footer_delete = "y delete";
    let footer_cancel = "Esc/n/q/Enter cancel";
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
        let lines = render_delete_footer(22, "y delete", "   ", "Esc/n/q/Enter cancel");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].width(), 8);
        assert_eq!(lines[1].width(), 20);
    }

    #[test]
    fn skip_zero_returns_full_string() {
        assert_eq!(skip_display_width("abcde", 0), "abcde");
    }

    #[test]
    fn skip_ascii_offset() {
        assert_eq!(skip_display_width("abcdefg", 3), "defg");
    }

    #[test]
    fn skip_exact_length_returns_empty() {
        assert_eq!(skip_display_width("abc", 3), "");
    }

    #[test]
    fn skip_exceeds_length_returns_empty() {
        assert_eq!(skip_display_width("abc", 10), "");
    }

    #[test]
    fn skip_unicode_multibyte() {
        // "café" = c(1) a(1) f(1) é(1 display width, 2 UTF-8 bytes)
        // skip 3 display chars -> should return "é"
        assert_eq!(skip_display_width("café", 3), "é");
    }

    #[test]
    fn skip_wide_char_full_column() {
        // '日' width=2; skip 2 columns → return "語"
        assert_eq!(skip_display_width("日語", 2), "語");
    }

    #[test]
    fn skip_bisects_wide_char_snaps_forward() {
        // '日' width=2; skip=1 bisects the wide char.
        // Policy: snap forward — skip the entire wide char, return "bc".
        assert_eq!(skip_display_width("日bc", 1), "bc");
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

fn centered_rect_percent(width_pct: u16, height_pct: u16, area: Rect) -> Rect {
    let width = (area.width as u32 * width_pct as u32 / 100) as u16;
    let height = (area.height as u32 * height_pct as u32 / 100) as u16;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

fn render_diff_overlay(state: &crate::tui::screens::diff::DiffViewerState, frame: &mut Frame) {
    use crate::core::git::DiffLineKind;
    use ratatui::widgets::Clear;

    let area = centered_rect_percent(90, 80, frame.area());
    frame.render_widget(Clear, area);

    let staged_label = if state.staged { "staged" } else { "unstaged" };
    let title = format!(
        " {}/{} \u{00b7} HEAD \u{00b7} {} ",
        state.repo_name, state.file_path, staged_label
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split inner into body + footer
    let sections = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

    match &state.diff {
        Ok(file_diff) => {
            let styled_lines: Vec<Line> = file_diff
                .lines
                .iter()
                .map(|dl| {
                    let style = match dl.kind {
                        DiffLineKind::Addition => theme::additions(),
                        DiffLineKind::Deletion => theme::deletions(),
                        DiffLineKind::HunkHeader => {
                            Style::default().fg(ratatui::style::Color::Cyan)
                        }
                        DiffLineKind::FileHeader => theme::muted(),
                        DiffLineKind::Context => Style::default(),
                        DiffLineKind::Binary => theme::muted(),
                    };
                    Line::from(Span::styled(dl.content.clone(), style))
                })
                .collect();
            let paragraph = Paragraph::new(styled_lines).scroll((state.scroll_offset, 0));
            frame.render_widget(paragraph, sections[0]);
        }
        Err(msg) => {
            frame.render_widget(
                Paragraph::new(msg.as_str()).style(theme::error()),
                sections[0],
            );
        }
    }

    let footer_hint = if state.staged {
        "  \u{2191}\u{2193} scroll \u{00b7} PgUp/PgDn page \u{00b7} s/space unstage \u{00b7} Esc close"
    } else {
        "  \u{2191}\u{2193} scroll \u{00b7} PgUp/PgDn page \u{00b7} s/space stage \u{00b7} Esc close"
    };
    frame.render_widget(
        Paragraph::new(footer_hint).style(theme::muted()),
        sections[1],
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
    let dialog_w = (frame.area().width * 70 / 100).max(50);
    let area = centered_rect_fixed(dialog_w, height, frame.area());
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

fn render_switch_branch_overlay(
    state: &crate::tui::screens::switch_branch::SwitchBranchState,
    frame: &mut Frame,
) {
    use crate::tui::screens::switch_branch::SwitchBranchStage;
    match &state.stage {
        SwitchBranchStage::PickStrategy => render_switch_strategy_picker(
            frame,
            &state.repo_name,
            state.strategy_idx,
            state.error.as_deref(),
            &state.recent_branches,
        ),
        SwitchBranchStage::EnterBranchName => render_text_input_dialog(
            "New Branch",
            "Branch name:",
            &state.branch_name_input,
            state.error.as_deref(),
            frame,
        ),
        SwitchBranchStage::PickBranch => {
            if let Some(ref picker) = state.branch_picker {
                crate::tui::widgets::fuzzy_picker::render(picker, frame);
            }
        }
    }
}

fn render_switch_strategy_picker(
    frame: &mut Frame,
    repo_name: &str,
    strategy_idx: usize,
    error: Option<&str>,
    recent_branches: &[crate::core::git::BranchInfo],
) {
    use ratatui::widgets::Clear;

    let has_error = error.is_some();
    let n = recent_branches.len();
    // Rows: "New branch..." (1) + if n>0: "Recent:" header (1) + n branches + "Show more" (1); else "Pick a branch..." (1)
    let branch_rows: u16 = 1 + if n > 0 { 1 + n as u16 + 1 } else { 1 };
    let height: u16 = (branch_rows + 2 + if has_error { 3 } else { 1 })
        .min(frame.area().height.saturating_sub(2));
    let dialog_w = (frame.area().width * 70 / 100).max(60);
    let area = centered_rect_fixed(dialog_w, height, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if has_error {
        theme::border_danger()
    } else {
        theme::border_focused()
    };
    let title = format!(" Switch Branch: {} ", repo_name);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = if has_error {
        Layout::vertical([
            Constraint::Length(branch_rows),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .split(inner)
    } else {
        Layout::vertical([Constraint::Length(branch_rows), Constraint::Min(0)]).split(inner)
    };

    let opt_max_w = dialog_w.saturating_sub(2 + 4) as usize;
    let mut items: Vec<ListItem> = Vec::new();

    // Index 0: New branch
    if strategy_idx == 0 {
        items.push(
            ListItem::new(format!(
                "> {}",
                truncate_for_width("New branch...", opt_max_w)
            ))
            .style(theme::selected()),
        );
    } else {
        items.push(ListItem::new(format!(
            "  {}",
            truncate_for_width("New branch...", opt_max_w)
        )));
    }

    if n > 0 {
        // Non-selectable "Recent:" header
        items.push(ListItem::new("  Recent:").style(theme::muted()));

        // Indices 1..=n: recent branches
        for (i, branch) in recent_branches.iter().enumerate() {
            let sel_idx = 1 + i;
            let time_str = crate::core::git::relative_time(branch.last_commit_time);
            let inner_w = dialog_w.saturating_sub(2) as usize;
            let time_w = line_width(&time_str);
            let max_name = inner_w.saturating_sub(6).saturating_sub(time_w + 2);
            let display_name = truncate_for_width(&branch.name, max_name);
            let display_w = line_width(&display_name);
            let content_w = inner_w.saturating_sub(6);
            let padding = content_w.saturating_sub(display_w + time_w);
            let line = format!("{}{}{}", display_name, " ".repeat(padding), time_str);
            if sel_idx == strategy_idx {
                items.push(ListItem::new(format!("  > {}", line)).style(theme::selected()));
            } else {
                items.push(ListItem::new(format!("    {}", line)));
            }
        }

        // Index n+1: "Show more..."
        let show_more_idx = 1 + n;
        if show_more_idx == strategy_idx {
            items.push(ListItem::new("  > Show more...").style(theme::selected()));
        } else {
            items.push(ListItem::new("    Show more..."));
        }
    } else {
        // No recent branches: index 1 = "Pick a branch..."
        if 1 == strategy_idx {
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

fn render_gitops_overlay(
    state: &crate::tui::screens::gitops::GitOpsState,
    spinner_tick: u64,
    frame: &mut Frame,
) {
    // Running stage: a network op (Phase 2: fetch) streaming live output.
    if state.stage == crate::tui::screens::gitops::GitOpsStage::Running {
        let dialog_w = (frame.area().width * 60 / 100).max(48);
        let dialog_h = (frame.area().height * 60 / 100)
            .max(10)
            .min(frame.area().height.saturating_sub(2));
        let title = format!(" Git: {} ({}) ", state.repo_name, state.branch);
        let inner = gitops_dialog(title, dialog_w, dialog_h, frame);

        // The Running stage is only entered via `start_network_op`, which
        // always sets `running_op`; the fallback is unreachable.
        let op = state
            .running_op
            .as_ref()
            .map(|op| op.label())
            .unwrap_or("Git op");
        let op_progressive = state
            .running_op
            .as_ref()
            .map(|op| op.progressive())
            .unwrap_or("Working");
        let header = match state.finished {
            None => {
                // Animated ellipsis, same cadence as the dashboard spinner.
                let frames = ["\u{b7}  ", "\u{b7}\u{b7} ", "\u{b7}\u{b7}\u{b7}"];
                let dots = frames[(spinner_tick as usize / 30) % 3];
                Span::styled(format!("{} {}", op_progressive, dots), theme::muted())
            }
            Some(true) => Span::styled(format!("\u{2713} {} complete", op), theme::success()),
            Some(false) => Span::styled(
                format!("\u{2717} {} failed (Esc/q to close)", op),
                theme::error(),
            ),
        };

        let sections = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
        frame.render_widget(Paragraph::new(Line::from(header)), sections[0]);

        // Show the tail of the output that fits in the viewport.
        let viewport = sections[1].height as usize;
        let start = state.output.len().saturating_sub(viewport);
        let lines: Vec<Line> = state.output[start..]
            .iter()
            .map(|l| Line::from(Span::styled(l.clone(), theme::muted())))
            .collect();
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            sections[1],
        );
        return;
    }

    // Log stage: read-only scrollable list of recent commits.
    if state.stage == crate::tui::screens::gitops::GitOpsStage::Log {
        let dialog_w = (frame.area().width * 70 / 100).max(56);
        let dialog_h = (frame.area().height * 70 / 100)
            .max(10)
            .min(frame.area().height.saturating_sub(2));
        let title = format!(" Git log: {} ({}) ", state.repo_name, state.branch);
        let inner = gitops_dialog(title, dialog_w, dialog_h, frame);

        // Reserve the last row for a key hint; the commit list fills the rest.
        let sections = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
        let list_area = sections[0];

        if state.commits.is_empty() {
            frame.render_widget(
                Paragraph::new("No commits.").style(theme::muted()),
                list_area,
            );
        } else {
            // Viewport-aware upper clamp: never start so far down that a blank
            // screenful shows below the last commit (the fix for the diff
            // viewer's over-scroll).
            let viewport_rows = list_area.height as usize;
            let max_start = state.commits.len().saturating_sub(viewport_rows);
            let start = (state.log_scroll as usize).min(max_start);
            let end = (start + viewport_rows).min(state.commits.len());
            let lines: Vec<Line> = state.commits[start..end]
                .iter()
                .map(|c| {
                    let text = format!(
                        "{}  {}  {}  {}",
                        c.short_hash,
                        crate::core::git::relative_time(c.time),
                        c.author,
                        c.subject
                    );
                    Line::from(Span::styled(text, theme::text()))
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), list_area);
        }

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "j/k scroll \u{00b7} Esc close",
                theme::muted(),
            ))),
            sections[1],
        );
        return;
    }

    // Rebase target picker: reuse the shared fuzzy branch picker (full-screen).
    if state.stage == crate::tui::screens::gitops::GitOpsStage::RebasePickTarget {
        if let Some(ref picker) = state.rebase_picker {
            crate::tui::widgets::fuzzy_picker::render(picker, frame);
        }
        return;
    }

    // Rebase pre-flight: branch state plus either a blocker or a ready prompt.
    if state.stage == crate::tui::screens::gitops::GitOpsStage::RebasePreflight {
        let dialog_w = (frame.area().width * 60 / 100).max(48);
        let dialog_h = 9u16.min(frame.area().height.saturating_sub(2));
        let title = format!(" Rebase: {} ({}) ", state.repo_name, state.branch);
        let inner = gitops_dialog(title, dialog_w, dialog_h, frame);

        let lines: Vec<Line> = match &state.rebase_block {
            Some(reason) => vec![
                Line::from(Span::styled(reason.clone(), theme::error())),
                Line::from(""),
                Line::from(Span::styled("Esc to go back", theme::muted())),
            ],
            None => vec![
                Line::from(Span::styled(
                    format!("Working tree clean on {}.", state.branch),
                    theme::text(),
                )),
                Line::from(Span::styled(
                    "Replay this branch's commits on top of another branch.",
                    theme::muted(),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Enter to choose a target \u{00b7} Esc to cancel",
                    theme::muted(),
                )),
            ],
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    // Rebase confirm: an ahead/behind preview plus the [y/N] prompt.
    if state.stage == crate::tui::screens::gitops::GitOpsStage::RebaseConfirm {
        let dialog_w = (frame.area().width * 60 / 100).max(48);
        // Seven logical lines, but the preview, the abort note, and the push
        // note each wrap to two rows at the minimum width — reserve room so
        // [y/N] is never clipped.
        let dialog_h = 13u16.min(frame.area().height.saturating_sub(2));
        let title = format!(" Rebase: {} ({}) ", state.repo_name, state.branch);
        let inner = gitops_dialog(title, dialog_w, dialog_h, frame);

        let onto = state.rebase_onto.as_deref().unwrap_or("?");
        let preview = match state.rebase_ahead_behind {
            Some((ahead, behind)) => format!(
                "{} commit(s) will be replayed. {} is {} commit(s) ahead of your branch.",
                ahead, onto, behind
            ),
            None => "Divergence unknown (could not compare with the target).".to_string(),
        };
        let lines = vec![
            Line::from(Span::styled(
                format!("Rebase {} onto {}?", state.branch, onto),
                theme::text(),
            )),
            Line::from(Span::styled(preview, theme::muted())),
            Line::from(""),
            Line::from(Span::styled(
                "On conflict the rebase is aborted and your branch restored.",
                theme::muted(),
            )),
            Line::from(Span::styled(
                "If this branch is already pushed, the next push will be rejected.",
                theme::muted(),
            )),
            Line::from(""),
            Line::from(Span::styled("Proceed?  [y/N]", theme::text())),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    // ConfirmPush stage: confirm publishing a branch that has no upstream.
    if state.stage == crate::tui::screens::gitops::GitOpsStage::ConfirmPush {
        let dialog_w = (frame.area().width * 60 / 100).max(48);
        let dialog_h = 7u16.min(frame.area().height.saturating_sub(2));
        let title = format!(" Git: {} ({}) ", state.repo_name, state.branch);
        let inner = gitops_dialog(title, dialog_w, dialog_h, frame);

        let prompt = format!(
            "Branch {} has no upstream. Push and set upstream to origin/{}?  [y/N]",
            state.branch, state.branch
        );
        frame.render_widget(Paragraph::new(prompt).wrap(Wrap { trim: false }), inner);
        return;
    }

    // Committing stage: staged-file summary above a single-line message input.
    if state.stage == crate::tui::screens::gitops::GitOpsStage::Committing {
        let dialog_w = (frame.area().width * 60 / 100).max(48);
        let staged_n = state.staged_files.len() as u16;
        // header + staged list + prompt + input + status row, plus the border.
        // The status row is always reserved because the layout below always
        // allocates it; counting it conditionally clipped the staged list by
        // one row whenever no status was shown.
        let content_rows = 1 + staged_n.max(1) + 1 + 1 + 1;
        let dialog_h = (content_rows + 2)
            .max(9)
            .min(frame.area().height.saturating_sub(2));
        let title = format!(" Git: {} ({}) ", state.repo_name, state.branch);
        let inner = gitops_dialog(title, dialog_w, dialog_h, frame);

        let sections = Layout::vertical([
            Constraint::Length(1), // header
            Constraint::Min(1),    // staged file list
            Constraint::Length(1), // prompt
            Constraint::Length(1), // input row
            Constraint::Length(1), // status / error
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new("Commit staged files:").style(theme::muted()),
            sections[0],
        );
        let staged: Vec<Line> = state
            .staged_files
            .iter()
            .map(|p| Line::from(Span::styled(format!("  {}", p), theme::text())))
            .collect();
        frame.render_widget(
            Paragraph::new(staged).wrap(Wrap { trim: false }),
            sections[1],
        );

        frame.render_widget(
            Paragraph::new("Message (Enter to commit, Esc to cancel):").style(theme::muted()),
            sections[2],
        );
        frame.render_widget(
            Paragraph::new(state.message_input.value()).style(theme::input_style()),
            sections[3],
        );
        if let Some(status) = &state.status {
            frame.render_widget(
                Paragraph::new(status.as_str()).style(theme::error()),
                sections[4],
            );
        }
        return;
    }

    let has_status = state.status.is_some();
    let content_rows = 6u16 + if has_status { 1 } else { 0 };
    let dialog_w = (frame.area().width * 50 / 100).max(40);
    let height: u16 = (content_rows + 2).min(frame.area().height.saturating_sub(2));
    let title = format!(" Git: {} ({}) ", state.repo_name, state.branch);
    let inner = gitops_dialog(title, dialog_w, height, frame);

    // Menu order and hotkeys come from the single source of truth in gitops.rs.
    let rows: Vec<ListItem> = crate::tui::screens::gitops::MENU
        .iter()
        .enumerate()
        .map(|(i, (keych, label))| {
            let enabled = state.item_enabled(i);
            let marker = if i == state.selected { "> " } else { "  " };
            let text = format!("{}{}  {}", marker, keych, label);
            let style = if !enabled {
                theme::dim_text()
            } else if i == state.selected {
                theme::selected()
            } else {
                Style::default()
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let sections = Layout::vertical([Constraint::Length(6), Constraint::Min(0)]).split(inner);
    frame.render_widget(List::new(rows), sections[0]);
    if let Some(status) = &state.status {
        frame.render_widget(
            Paragraph::new(status.as_str()).style(theme::muted()),
            sections[1],
        );
    }
}

/// Shared chrome for every git-ops overlay stage: center a `dialog_w` x
/// `dialog_h` dialog, clear it, draw the rounded focused border with `title`,
/// and return the inner content area.
fn gitops_dialog(title: String, dialog_w: u16, dialog_h: u16, frame: &mut Frame) -> Rect {
    use ratatui::widgets::Clear;

    let area = centered_rect_fixed(dialog_w, dialog_h, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
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

use crate::tui::app::{App, Pane, Screen};
use crate::tui::theme;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Row, Table,
        TableState, Wrap,
    },
    Frame,
};

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
        Span::styled(
            format!("v{}", env!("CARGO_PKG_VERSION")),
            theme::muted(),
        ),
    ]);
    frame.render_widget(Paragraph::new(title), area);
}

fn render_main(app: &App, frame: &mut Frame, area: Rect) {
    let panes =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).split(area);

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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(format!(" {} ", ws_name));

    let repos = app
        .selected_workspace()
        .map(|ws| ws.repos.as_slice())
        .unwrap_or(&[]);

    if repos.is_empty() {
        frame.render_widget(Paragraph::new("  No repos").block(block), area);
        return;
    }

    let rows: Vec<Row> = repos
        .iter()
        .map(|r| {
            let status_style = if r.status.modified + r.status.staged > 0 {
                theme::warn()
            } else {
                theme::status_clean()
            };
            let status_str = if r.status.modified + r.status.staged > 0 {
                format!("{}m {}s", r.status.modified, r.status.staged)
            } else {
                "clean".to_string()
            };
            let ab = if r.ahead + r.behind > 0 {
                format!("+{} -{}", r.ahead, r.behind)
            } else {
                String::new()
            };
            Row::new(vec![
                ratatui::text::Span::raw(r.name.clone()),
                ratatui::text::Span::styled(r.branch.clone(), theme::branch()),
                ratatui::text::Span::styled(status_str, status_style),
                ratatui::text::Span::styled(ab, theme::warn()),
            ])
        })
        .collect();

    let header = Row::new(vec!["REPO", "BRANCH", "STATUS", "+/-"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
            Constraint::Percentage(10),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(theme::highlight_row());

    let mut state = TableState::default();
    if !repos.is_empty() && focused {
        state.select(Some(app.selected_repo));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    if let Some(msg) = &app.status_message {
        frame.render_widget(Paragraph::new(msg.as_str()).style(theme::muted()), area);
        return;
    }

    let sep = || Span::styled("  ·  ", theme::muted());
    let key = |k: &'static str| Span::styled(k, theme::text());
    let act = |a: &'static str| Span::styled(a, theme::muted());

    let bar = Line::from(vec![
        key("enter"), act(" go"),
        sep(),
        key("c"), act(" create"),
        sep(),
        key("a"), act(" add"),
        sep(),
        key("d"), act(" delete"),
        sep(),
        key("r"), act(" refresh"),
        sep(),
        key("/"), act(" search"),
        sep(),
        key("S"), act(" config"),
        sep(),
        key("q"), act(" quit"),
    ]);
    frame.render_widget(Paragraph::new(bar), area);
}

fn render_create_overlay(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    use crate::tui::screens::create::CreateStage;
    match &state.stage {
        CreateStage::PickRepos => {
            crate::tui::widgets::fuzzy_picker::render(&state.picker, frame);
        }
        CreateStage::NameWorkspace => render_name_input(state, frame),
        CreateStage::PickBranchStrategy => render_branch_strategy(state, frame),
        CreateStage::PickBranch => {
            if let Some(ref picker) = state.branch_picker {
                crate::tui::widgets::fuzzy_picker::render(picker, frame);
            }
        }
        CreateStage::Creating => render_creating_progress(state, frame),
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
        Paragraph::new(format!("> {}", state.ws_name.value()))
            .style(theme::input_style()),
        sections[1],
    );
    if let Some(err) = &state.error {
        frame.render_widget(
            Paragraph::new(err.as_str()).style(theme::error()),
            sections[2],
        );
    }
}

fn render_branch_strategy(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let has_error = state.error.is_some();
    // 2 borders + 4 options + 1 padding + (1 sep + 2 error) when error present
    let height: u16 = if has_error { 10 } else { 8 };
    let area = centered_rect_fixed(62, height, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if has_error { theme::border_danger() } else { theme::border_focused() };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(" Branch Strategy ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Layout: 4 option rows, then separator + error only when there's an error
    let sections = if has_error {
        Layout::vertical([
            Constraint::Length(4), // options
            Constraint::Length(1), // separator
            Constraint::Length(2), // error (wraps to 2 lines)
        ])
        .split(inner)
    } else {
        Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).split(inner)
    };

    let options = [
        format!("New branch '{}'", state.ws_name.value()),
        format!("Existing branch '{}' (if present)", state.ws_name.value()),
        "Detached HEAD".to_string(),
        "Pick a branch...".to_string(),
    ];

    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            if i == state.branch_strategy_idx {
                ListItem::new(format!("> {}", opt)).style(theme::selected())
            } else {
                ListItem::new(format!("  {}", opt))
            }
        })
        .collect();

    frame.render_widget(List::new(items), sections[0]);

    if let Some(err) = &state.error {
        frame.render_widget(
            Paragraph::new(format!("\u{26a0}  {}", err))
                .style(theme::error())
                .wrap(Wrap { trim: false }),
            sections[2],
        );
    }
}

fn render_creating_progress(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let area = centered_rect_fixed(60, 15, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(" Creating Workspace ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = state
        .progress
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

    if let Some(err) = &state.error {
        frame.render_widget(
            Paragraph::new(format!("Error: {}  [ESC to dismiss]", err))
                .style(theme::error()),
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
        AddStage::PickBranchStrategy => render_add_branch_strategy(state, frame),
        AddStage::PickBranch => {
            if let Some(ref picker) = state.branch_picker {
                crate::tui::widgets::fuzzy_picker::render(picker, frame);
            }
        }
        AddStage::Creating => render_add_progress(state, frame),
    }
}

fn render_add_branch_strategy(state: &crate::tui::screens::add::AddState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let has_error = state.error.is_some();
    let height: u16 = if has_error { 10 } else { 8 };
    let area = centered_rect_fixed(62, height, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if has_error { theme::border_danger() } else { theme::border_focused() };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(" Branch Strategy ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = if has_error {
        Layout::vertical([
            Constraint::Length(4),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .split(inner)
    } else {
        Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).split(inner)
    };

    let options = [
        format!("New branch '{}'", state.workspace_name),
        format!("Existing branch '{}' (if present)", state.workspace_name),
        "Detached HEAD".to_string(),
        "Pick a branch...".to_string(),
    ];

    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            if i == state.branch_strategy_idx {
                ListItem::new(format!("> {}", opt)).style(theme::selected())
            } else {
                ListItem::new(format!("  {}", opt))
            }
        })
        .collect();

    frame.render_widget(List::new(items), sections[0]);

    if let Some(err) = &state.error {
        frame.render_widget(
            Paragraph::new(format!("\u{26a0}  {}", err))
                .style(theme::error())
                .wrap(Wrap { trim: false }),
            sections[2],
        );
    }
}

fn render_add_progress(state: &crate::tui::screens::add::AddState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let area = centered_rect_fixed(60, 15, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(" Adding Repos ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = state
        .progress
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

    if let Some(err) = &state.error {
        frame.render_widget(
            Paragraph::new(format!("Error: {}  [ESC to dismiss]", err))
                .style(theme::error()),
            sections[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new("Done! [ENTER to continue]").style(theme::success()),
            sections[1],
        );
    }
}

fn render_delete_confirm(state: &crate::tui::screens::delete::DeleteState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let height = (5 + state.repo_names.len()).min(20) as u16;
    let area = centered_rect_fixed(44, height, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_danger())
        .title(" Confirm Delete ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!("Remove workspace '{}'?", state.workspace_name),
            theme::text().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for name in &state.repo_names {
        lines.push(Line::from(Span::styled(
            format!("  {}  (clean)", name),
            theme::dim_text(),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  [y] confirm", theme::success()),
        Span::raw("   "),
        Span::styled("[n/ESC] cancel", theme::muted()),
    ]));

    frame.render_widget(Paragraph::new(lines), inner);
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
        .flat_map(|_| [Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
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
                if is_focused { theme::selected() } else { theme::text() },
            ))
        } else {
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(
                    field.label,
                    if is_focused { theme::selected() } else { theme::text() },
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

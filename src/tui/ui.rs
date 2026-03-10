use crate::tui::app::{App, Pane, Screen};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Row, Table, TableState},
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
        Constraint::Length(1),  // title
        Constraint::Min(0),     // main
        Constraint::Length(1),  // status bar
    ])
    .split(area);

    render_title(frame, outer[0]);
    render_main(app, frame, outer[1]);
    render_status_bar(app, frame, outer[2]);
}

fn render_title(frame: &mut Frame, area: Rect) {
    let title = Line::from(vec![
        Span::styled(" space ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(title), area);
}

fn render_main(app: &App, frame: &mut Frame, area: Rect) {
    let panes = Layout::horizontal([
        Constraint::Percentage(30),
        Constraint::Percentage(70),
    ])
    .split(area);

    render_workspace_list(app, frame, panes[0]);
    render_repo_table(app, frame, panes[1]);
}

fn render_workspace_list(app: &App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Pane::Left;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
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

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" WORKSPACES "),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.workspaces.is_empty() {
        state.select(Some(app.selected_ws));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_repo_table(app: &App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Pane::Right;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let ws_name = app
        .selected_workspace()
        .map(|ws| ws.name.as_str())
        .unwrap_or("");

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(" {} ", ws_name));

    let repos = app
        .selected_workspace()
        .map(|ws| ws.repos.as_slice())
        .unwrap_or(&[]);

    if repos.is_empty() {
        frame.render_widget(
            Paragraph::new("  No repos").block(block),
            area,
        );
        return;
    }

    let rows: Vec<Row> = repos
        .iter()
        .map(|r| {
            let status_style = if r.status.modified + r.status.staged > 0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Green)
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
                ratatui::text::Span::styled(r.branch.clone(), Style::default().fg(Color::Green)),
                ratatui::text::Span::styled(status_str, status_style),
                ratatui::text::Span::styled(ab, Style::default().fg(Color::Yellow)),
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
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = TableState::default();
    if !repos.is_empty() && focused {
        state.select(Some(app.selected_repo));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    let msg = app.status_message.as_deref().unwrap_or(
        "<enter> go  <c> create  <a> add  <d> delete  <r> refresh  </> search  <q> quit",
    );
    let bar = Paragraph::new(msg).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(bar, area);
}

fn render_create_overlay(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    use crate::tui::screens::create::CreateStage;
    match &state.stage {
        CreateStage::PickRepos => {
            crate::tui::widgets::fuzzy_picker::render(&state.picker, frame);
        }
        CreateStage::NameWorkspace => render_name_input(state, frame),
        CreateStage::PickBranchStrategy => render_branch_strategy(state, frame),
        CreateStage::Creating => render_creating_progress(state, frame),
    }
}

fn render_name_input(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let area = centered_rect_fixed(50, 7, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
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
        Paragraph::new("Enter workspace name:").style(Style::default().fg(Color::White)),
        sections[0],
    );
    frame.render_widget(
        Paragraph::new(format!("> {}", state.ws_name.value()))
            .style(Style::default().fg(Color::Cyan)),
        sections[1],
    );
    if let Some(err) = &state.error {
        frame.render_widget(
            Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red)),
            sections[2],
        );
    }
}

fn render_branch_strategy(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let area = centered_rect_fixed(50, 9, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Branch Strategy ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let options = [
        format!("New branch '{}'", state.ws_name.value()),
        format!("Existing branch '{}' (if present)", state.ws_name.value()),
        "Detached HEAD".to_string(),
    ];

    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            if i == state.branch_strategy_idx {
                ListItem::new(format!("> {}", opt))
                    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            } else {
                ListItem::new(format!("  {}", opt))
            }
        })
        .collect();

    frame.render_widget(List::new(items), inner);
}

fn render_creating_progress(state: &crate::tui::screens::create::CreateState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let area = centered_rect_fixed(60, 15, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Creating Workspace ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = state
        .progress
        .iter()
        .map(|l| {
            if l.starts_with("  \u{2713}") {
                Line::from(Span::styled(l.clone(), Style::default().fg(Color::Green)))
            } else if l.starts_with("  \u{2717}") {
                Line::from(Span::styled(l.clone(), Style::default().fg(Color::Red)))
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
                .style(Style::default().fg(Color::Red)),
            sections[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new("Done! [ENTER to continue]")
                .style(Style::default().fg(Color::Green)),
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
        AddStage::Creating => render_add_progress(state, frame),
    }
}

fn render_add_branch_strategy(state: &crate::tui::screens::add::AddState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let area = centered_rect_fixed(50, 9, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Branch Strategy ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let options = [
        format!("New branch '{}'", state.workspace_name),
        format!("Existing branch '{}' (if present)", state.workspace_name),
        "Detached HEAD".to_string(),
    ];

    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            if i == state.branch_strategy_idx {
                ListItem::new(format!("> {}", opt))
                    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            } else {
                ListItem::new(format!("  {}", opt))
            }
        })
        .collect();

    frame.render_widget(List::new(items), inner);
}

fn render_add_progress(state: &crate::tui::screens::add::AddState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let area = centered_rect_fixed(60, 15, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Adding Repos ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = state
        .progress
        .iter()
        .map(|l| {
            if l.starts_with("  \u{2713}") {
                Line::from(Span::styled(l.clone(), Style::default().fg(Color::Green)))
            } else if l.starts_with("  \u{2717}") {
                Line::from(Span::styled(l.clone(), Style::default().fg(Color::Red)))
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
                .style(Style::default().fg(Color::Red)),
            sections[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new("Done! [ENTER to continue]")
                .style(Style::default().fg(Color::Green)),
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
        .border_style(Style::default().fg(Color::Red))
        .title(" Confirm Delete ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!("Remove workspace '{}'?", state.workspace_name),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for name in &state.repo_names {
        lines.push(Line::from(Span::styled(
            format!("  {}  (clean)", name),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  [y] confirm", Style::default().fg(Color::Green)),
        Span::raw("   "),
        Span::styled("[n/ESC] cancel", Style::default().fg(Color::DarkGray)),
    ]));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_config_editor(state: &crate::tui::screens::config::ConfigState, frame: &mut Frame) {
    use ratatui::widgets::Clear;
    let area = frame.area();
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Configuration  ESC=save and exit ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Build vertical layout: 2 rows per field + 1 hint line
    let field_rows: u16 = 2;
    let mut constraints: Vec<Constraint> = state.fields.iter()
        .map(|_| Constraint::Length(field_rows))
        .collect();
    constraints.push(Constraint::Min(0)); // spacer
    constraints.push(Constraint::Length(1)); // hint line
    let sections = Layout::vertical(constraints).split(inner);

    for (i, field) in state.fields.iter().enumerate() {
        let row_area = sections[i];
        let sub = Layout::horizontal([
            Constraint::Length(22),
            Constraint::Min(0),
        ]).split(row_area);

        let is_focused = i == state.focused;
        let label_style = if is_focused {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        frame.render_widget(
            Paragraph::new(format!("{}:", field.label)).style(label_style),
            sub[0],
        );

        if is_focused && state.editing {
            frame.render_widget(
                Paragraph::new(format!("> {}", state.input.value()))
                    .style(Style::default().fg(Color::Yellow)),
                sub[1],
            );
        } else {
            let value_style = if is_focused {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            frame.render_widget(
                Paragraph::new(field.value.clone()).style(value_style),
                sub[1],
            );
        }
    }

    // Hint line is the last section
    let hint_idx = state.fields.len() + 1;
    frame.render_widget(
        Paragraph::new("↑↓ navigate  ENTER edit  ESC save & exit")
            .style(Style::default().fg(Color::DarkGray)),
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

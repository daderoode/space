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

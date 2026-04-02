use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::{App, WizardField};

pub fn render_wizard(frame: &mut Frame, app: &App) {
    let area = centered_rect(62, 22, frame.area());
    frame.render_widget(Clear, area);

    let outer_block = Block::default()
        .title(" himalaya-tui — first run ")
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        );

    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    let Some(wizard) = &app.wizard else {
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // description
            Constraint::Length(1), // hint line
            Constraint::Length(1), // spacer
            Constraint::Length(3), // URI field
            Constraint::Length(1), // spacer
            Constraint::Length(3), // Username field
            Constraint::Length(1), // spacer
            Constraint::Length(3), // Password field
            Constraint::Length(1), // spacer
            Constraint::Length(1), // error / connecting
            Constraint::Fill(1),   // flexible spacer
            Constraint::Length(1), // key hints
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new("No configuration found. Enter your server details to connect."),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(
            "Supported: imaps://user@host  imap://user@host  https://host  jmaps://host",
        )
        .style(Style::default().fg(Color::DarkGray)),
        chunks[1],
    );

    render_field(
        frame,
        chunks[3],
        "URI",
        &wizard.uri,
        wizard.active_field == WizardField::Uri,
    );
    render_field(
        frame,
        chunks[5],
        "Username",
        &wizard.username,
        wizard.active_field == WizardField::Username,
    );

    let masked = "•".repeat(wizard.password.chars().count());
    render_field(
        frame,
        chunks[7],
        "Password",
        &masked,
        wizard.active_field == WizardField::Password,
    );

    if wizard.connecting {
        frame.render_widget(
            Paragraph::new("Connecting…").style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            chunks[9],
        );
    } else if let Some(err) = &wizard.error {
        frame.render_widget(
            Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red)),
            chunks[9],
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" next field   "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" connect   "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" quit"),
        ])),
        chunks[11],
    );
}

fn render_field(frame: &mut Frame, area: Rect, label: &str, value: &str, active: bool) {
    let border_style = if active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Gray)
    };

    let block = Block::default()
        .title(format!(" {label} "))
        .borders(Borders::ALL)
        .border_style(border_style);

    let display = if active {
        format!("{value}█")
    } else {
        value.to_string()
    };

    frame.render_widget(Paragraph::new(display).block(block), area);
}

fn centered_rect(width: u16, height: u16, r: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(r);

    let x = r.x + r.width.saturating_sub(width) / 2;
    let actual_width = width.min(r.width);

    Rect::new(x, vertical[1].y, actual_width, vertical[1].height)
}

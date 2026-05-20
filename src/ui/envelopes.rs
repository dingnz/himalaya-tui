use io_email::{envelope::Envelope, flag::Flag};
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

use super::get_border_style;
use crate::app::{App, Panel};

pub fn render_envelopes(frame: &mut Frame, app: &App, area: Rect) {
    let header_cells = ["Flags", "Subject", "From", "Date"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells)
        .style(Style::default().fg(Color::Yellow))
        .height(1);

    let rows: Vec<Row> = app
        .envelopes
        .iter()
        .enumerate()
        .map(|(i, envelope)| {
            let style = if i == app.envelope_index && app.active_panel == Panel::Envelopes {
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else if envelope.flags.contains(&Flag::Seen) {
                Style::default().fg(Color::Gray)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };

            let cells = vec![
                Cell::from(format_flags(envelope)),
                Cell::from(envelope.subject.clone()),
                Cell::from(truncate(&format_from(envelope), 20)),
                Cell::from(truncate(&format_date(envelope), 6)),
            ];

            Row::new(cells).style(style)
        })
        .collect();

    let block = Block::default()
        .title(format!(
            " Envelopes{} ",
            app.selected_mailbox_name()
                .map(|m| {
                    let total_pages = app.total_pages();
                    if total_pages > 1 {
                        format!(" - {} ({}/{})", m, app.envelope_page + 1, total_pages)
                    } else {
                        format!(" - {}", m)
                    }
                })
                .unwrap_or_default()
        ))
        .borders(Borders::ALL)
        .border_style(get_border_style(app, Panel::Envelopes));

    let widths = [
        Constraint::Length(6),
        Constraint::Min(10),
        Constraint::Length(20),
        Constraint::Length(6),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_widget(table, area);
}

fn format_flags(envelope: &Envelope) -> String {
    let mut s = String::new();
    s.push(if envelope.flags.contains(&Flag::Seen) {
        ' '
    } else {
        '*'
    });
    s.push(if envelope.flags.contains(&Flag::Answered) {
        '↩'
    } else {
        ' '
    });
    s.push(if envelope.flags.contains(&Flag::Flagged) {
        '!'
    } else {
        ' '
    });
    s.push(if envelope.flags.contains(&Flag::Draft) {
        'D'
    } else {
        ' '
    });
    s
}

fn format_from(envelope: &Envelope) -> String {
    envelope
        .from
        .first()
        .map(|addr| addr.name.clone().unwrap_or_else(|| addr.email.clone()))
        .unwrap_or_default()
}

fn format_date(envelope: &Envelope) -> String {
    envelope
        .date
        .map(|d| d.format("%d %b").to_string())
        .unwrap_or_default()
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max_len - 3).collect::<String>())
    }
}

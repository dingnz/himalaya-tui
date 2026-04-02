use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use super::get_border_style;
use crate::app::{App, Panel};

pub fn render_mailboxes(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .mailboxes
        .iter()
        .enumerate()
        .map(|(i, mailbox)| {
            let style = if i == app.mailbox_index && app.active_panel == Panel::Mailboxes {
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else if Some(&mailbox.name) == app.selected_mailbox.as_ref() {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let prefix = if mailbox.subscribed { "* " } else { "  " };
            let content = format!("{}{}", prefix, mailbox.name);
            ListItem::new(Line::from(Span::styled(content, style)))
        })
        .collect();

    let block = Block::default()
        .title(" Mailboxes ")
        .borders(Borders::ALL)
        .border_style(get_border_style(app, Panel::Mailboxes));

    let list = List::new(items).block(block);

    frame.render_widget(list, area);
}

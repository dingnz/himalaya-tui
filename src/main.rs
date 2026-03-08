use std::{fs::File, io, path::PathBuf};

use anyhow::Result;
use edtui::{actions::system_editor, EditorEventHandler};
use mml::message::compiler::MmlCompilerBuilder;
use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
            KeyModifiers,
        },
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    Terminal,
};

use himalaya_tui::app::{App, ComposeAction, Dialog, EnvelopeAction, Panel};
use himalaya_tui::ui;

#[cfg(feature = "imap")]
use himalaya_tui::imap;
#[cfg(feature = "smtp")]
use himalaya_tui::smtp;

fn main() -> Result<()> {
    let log_file = File::create("/tmp/himalaya-tui.log")?;
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Trace,
        simplelog::Config::default(),
        log_file,
    )?;

    let config_paths = get_config_paths();
    let account_name = std::env::args().nth(1);

    let mut app = App::new(&config_paths, account_name.as_deref())?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    #[cfg(feature = "imap")]
    {
        app.set_status("Connecting to IMAP server...");
        terminal.draw(|f| ui::render(f, &mut app))?;

        match imap::fetch_mailboxes(&app.imap_config) {
            Ok(mailboxes) => {
                app.set_mailboxes(mailboxes);
                if let Some(ref mailbox) = app.selected_mailbox.clone() {
                    match imap::fetch_envelopes(&app.imap_config, mailbox) {
                        Ok(envelopes) => app.set_envelopes(envelopes),
                        Err(e) => app.set_status(format!("Error: {}", e)),
                    }
                }
            }
            Err(e) => app.set_status(format!("Error: {}", e)),
        }
    }

    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = result {
        eprintln!("Error: {err:?}");
    }

    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    let mut editor_handler = EditorEventHandler::default();

    while app.running {
        terminal.draw(|f| ui::render(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Handle dialog if open
            if let Some(dialog) = app.dialog {
                match dialog {
                    Dialog::Envelope => handle_envelope_dialog(app, key.code),
                    Dialog::Compose => handle_compose_dialog(app, key.code),
                    #[cfg(feature = "imap")]
                    Dialog::CopyTo => handle_copy_to_dialog(app, key.code),
                    #[cfg(feature = "imap")]
                    Dialog::MoveTo => handle_move_to_dialog(app, key.code),
                    #[cfg(feature = "imap")]
                    Dialog::Delete => handle_delete_dialog(app, key.code),
                    #[cfg(not(feature = "imap"))]
                    Dialog::CopyTo | Dialog::MoveTo | Dialog::Delete => {
                        if key.code == KeyCode::Esc {
                            app.close_dialog();
                        }
                    }
                }
                continue;
            }

            // Handle compose mode
            if app.active_panel == Panel::Compose {
                if key.code == KeyCode::Esc {
                    app.open_dialog(Dialog::Compose);
                    continue;
                }

                // Forward to edtui
                editor_handler.on_key_event(key, &mut app.editor_state);

                // Check if system editor was requested (Alt+e)
                if system_editor::is_pending(&app.editor_state) {
                    system_editor::open(&mut app.editor_state, terminal)?;
                    execute!(terminal.backend_mut(), EnableMouseCapture)?;
                }

                continue;
            }

            // Ctrl+C: new composition
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.start_compose();
                continue;
            }

            // Normal mode key handling
            match key.code {
                KeyCode::Esc => {
                    if app.previewing_compose {
                        app.close_preview();
                    } else if !app.close_current() {
                        app.quit();
                    }
                }
                KeyCode::Tab => app.toggle_panel(),
                KeyCode::Down => app.next_item(),
                KeyCode::Up => app.previous_item(),
                KeyCode::Enter => {
                    #[cfg(feature = "imap")]
                    handle_enter(app);
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn handle_envelope_dialog(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let action = app.get_selected_envelope_action();
            app.close_dialog();

            #[cfg(feature = "imap")]
            match action {
                EnvelopeAction::Read => handle_read_message(app),
                EnvelopeAction::Reply => handle_reply(app, false),
                EnvelopeAction::ReplyAll => handle_reply(app, true),
                EnvelopeAction::Forward => handle_forward(app),
                EnvelopeAction::Copy => app.open_dialog(Dialog::CopyTo),
                EnvelopeAction::Move => app.open_dialog(Dialog::MoveTo),
                EnvelopeAction::Delete => app.open_dialog(Dialog::Delete),
            }

            #[cfg(not(feature = "imap"))]
            {
                let _ = action;
                app.set_status("IMAP feature not enabled");
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

fn handle_compose_dialog(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let action = app.get_selected_compose_action();
            match action {
                ComposeAction::Send => {
                    #[cfg(feature = "smtp")]
                    {
                        let Some(smtp_config) = app.smtp_config.clone() else {
                            app.set_status("SMTP not configured");
                            return;
                        };

                        let content = app.get_compose_content();
                        app.set_status("Compiling message...");

                        match MmlCompilerBuilder::new().build(&content) {
                            Ok(compiler) => match compiler.compile() {
                                Ok(result) => match result.into_vec() {
                                    Ok(mime_bytes) => {
                                        app.set_status("Sending message...");
                                        match smtp::send_message(&smtp_config, &mime_bytes) {
                                            Ok(()) => {
                                                app.set_status("Message sent");
                                                app.cancel_compose();
                                            }
                                            Err(e) => app.set_status(format!("Send error: {e}")),
                                        }
                                    }
                                    Err(e) => app.set_status(format!("Error: {e}")),
                                },
                                Err(e) => app.set_status(format!("Compile error: {e}")),
                            },
                            Err(e) => app.set_status(format!("Parse error: {e}")),
                        }
                    }
                    #[cfg(not(feature = "smtp"))]
                    {
                        app.set_status("SMTP feature not enabled");
                    }
                }
                ComposeAction::Preview => {
                    let content = app.get_compose_content();
                    match MmlCompilerBuilder::new().build(&content) {
                        Ok(compiler) => match compiler.compile() {
                            Ok(result) => match result.into_string() {
                                Ok(mime) => {
                                    app.close_dialog();
                                    app.preview_compose(mime);
                                }
                                Err(e) => app.set_status(format!("Error: {e}")),
                            },
                            Err(e) => app.set_status(format!("Error compiling: {e}")),
                        },
                        Err(e) => app.set_status(format!("Error parsing: {e}")),
                    }
                }
                ComposeAction::SaveToDrafts => {
                    #[cfg(feature = "imap")]
                    {
                        let content = app.get_compose_content();
                        app.set_status("Saving to Drafts...");
                        match imap::save_to_drafts(&app.imap_config, &content) {
                            Ok(_) => {
                                app.set_status("Saved to Drafts");
                                app.cancel_compose();
                            }
                            Err(e) => app.set_status(format!("Error: {}", e)),
                        }
                    }
                    #[cfg(not(feature = "imap"))]
                    {
                        app.set_status("IMAP feature not enabled");
                        app.cancel_compose();
                    }
                }
                ComposeAction::Cancel => {
                    app.close_dialog();
                }
            }
        }
        KeyCode::Esc => {
            app.cancel_compose();
        }
        _ => {}
    }
}

#[cfg(feature = "imap")]
fn handle_enter(app: &mut App) {
    match app.active_panel {
        Panel::Mailboxes => {
            app.select_mailbox();
            if let Some(ref mailbox) = app.selected_mailbox {
                match imap::fetch_envelopes(&app.imap_config, mailbox) {
                    Ok(envelopes) => app.set_envelopes(envelopes),
                    Err(e) => app.set_status(format!("Error: {}", e)),
                }
            }
        }
        Panel::Envelopes => {
            if app.get_selected_envelope().is_some() {
                app.open_dialog(Dialog::Envelope);
            }
        }
        Panel::Message => {
            app.close_bottom_panel();
        }
        Panel::Compose => {}
    }
}

#[cfg(feature = "imap")]
fn handle_read_message(app: &mut App) {
    if let (Some(envelope), Some(mailbox)) = (
        app.get_selected_envelope().cloned(),
        app.selected_mailbox.clone(),
    ) {
        app.set_status(format!("Loading message {}...", envelope.uid));
        match imap::fetch_message(&app.imap_config, &mailbox, envelope.uid) {
            Ok(content) => app.show_message(content),
            Err(e) => app.set_status(format!("Error: {}", e)),
        }
    }
}

#[cfg(feature = "imap")]
fn handle_reply(app: &mut App, reply_all: bool) {
    if let (Some(envelope), Some(mailbox)) = (
        app.get_selected_envelope().cloned(),
        app.selected_mailbox.clone(),
    ) {
        app.set_status(format!("Loading message {}...", envelope.uid));
        match imap::fetch_raw_message(&app.imap_config, &mailbox, envelope.uid) {
            Ok(raw) => app.start_reply(&raw, reply_all),
            Err(e) => app.set_status(format!("Error: {}", e)),
        }
    }
}

#[cfg(feature = "imap")]
fn handle_delete_dialog(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let confirmed = app.dialog_index == 0;
            app.close_dialog();

            if !confirmed {
                return;
            }

            if let (Some(envelope), Some(mailbox)) = (
                app.get_selected_envelope().cloned(),
                app.selected_mailbox.clone(),
            ) {
                app.set_status(format!("Deleting message {}...", envelope.uid));
                match imap::delete_message(&app.imap_config, &mailbox, envelope.uid) {
                    Ok(_) => {
                        app.flag_selected_envelope("\\Deleted");
                        app.set_status("Message flagged as deleted");
                    }
                    Err(e) => app.set_status(format!("Error: {}", e)),
                }
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

#[cfg(feature = "imap")]
fn handle_copy_to_dialog(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let target = app.mailboxes.get(app.dialog_index).map(|m| m.name.clone());
            app.close_dialog();

            if let (Some(target), Some(envelope), Some(mailbox)) = (
                target,
                app.get_selected_envelope().cloned(),
                app.selected_mailbox.clone(),
            ) {
                app.set_status(format!("Copying to {}...", target));
                match imap::copy_message(&app.imap_config, &mailbox, envelope.uid, &target) {
                    Ok(_) => app.set_status(format!("Copied to {}", target)),
                    Err(e) => app.set_status(format!("Error: {}", e)),
                }
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

#[cfg(feature = "imap")]
fn handle_move_to_dialog(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let target = app.mailboxes.get(app.dialog_index).map(|m| m.name.clone());
            app.close_dialog();

            if let (Some(target), Some(envelope), Some(mailbox)) = (
                target,
                app.get_selected_envelope().cloned(),
                app.selected_mailbox.clone(),
            ) {
                app.set_status(format!("Moving to {}...", target));
                match imap::move_message(&app.imap_config, &mailbox, envelope.uid, &target) {
                    Ok(_) => {
                        app.remove_selected_envelope();
                        app.set_status(format!("Moved to {}", target));
                    }
                    Err(e) => app.set_status(format!("Error: {}", e)),
                }
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

#[cfg(feature = "imap")]
fn handle_forward(app: &mut App) {
    if let (Some(envelope), Some(mailbox)) = (
        app.get_selected_envelope().cloned(),
        app.selected_mailbox.clone(),
    ) {
        app.set_status(format!("Loading message {}...", envelope.uid));
        match imap::fetch_raw_message(&app.imap_config, &mailbox, envelope.uid) {
            Ok(raw) => app.start_forward(&raw),
            Err(e) => app.set_status(format!("Error: {}", e)),
        }
    }
}

fn get_config_paths() -> Vec<PathBuf> {
    if let Ok(paths) = std::env::var("HIMALAYA_CONFIG") {
        paths
            .split(':')
            .filter_map(|p| {
                let expanded = shellexpand::full(p).ok()?;
                Some(PathBuf::from(expanded.as_ref()))
            })
            .collect()
    } else {
        Vec::new()
    }
}

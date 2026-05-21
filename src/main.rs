// This file is part of Himalaya TUI, a TUI to manage emails.
//
// Copyright (C) 2025-2026 soywod <pimalaya.org@posteo.net>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more
// details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use std::{fs::File, io, path::PathBuf, time::Duration};

use anyhow::{Result, anyhow};
use clap::Parser;
use edtui::actions::{Execute, OpenSystemEditor, system_editor};
#[cfg(all(feature = "imap", feature = "smtp", feature = "jmap"))]
use himalaya_tui::wizard;
use himalaya_tui::{
    app::{App, ComposeAction, Dialog, EnvelopeAction, Keybinds, Panel},
    cli::HimalayaTuiCli,
    config::{AccountConfig, Config},
    ui,
};
use io_email::{client::EmailClientStd, flag::Flag};
use mml::compiler::message::MmlCompilerBuilder;
use pimalaya_cli::printer::StdoutPrinter;
use pimalaya_config::toml::TomlConfig;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
            KeyModifiers,
        },
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let cli = HimalayaTuiCli::parse();

    // Auxiliary subcommands (completions, manuals) run before the TUI
    // ever starts and print to stdout.
    if let Some(command) = cli.command {
        let mut printer = StdoutPrinter::new(&cli.json);
        return command.execute(&mut printer);
    }

    let log_file = File::create("/tmp/himalaya-tui.log")?;
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Trace,
        simplelog::Config::default(),
        log_file,
    )?;

    // Resolve config (loaded from disk if present, otherwise built
    // in-memory by the wizard) in normal terminal mode so inquire
    // prompts can render. The TUI's alternate screen kicks in after
    // the client is built.
    let (mut app, client) = match load_then_connect(
        &cli.config_paths,
        cli.account.as_deref(),
        cli.no_config,
        cli.from.as_deref(),
        cli.keybinds,
    ) {
        Ok(setup) => setup,
        Err(err) => {
            eprintln!("Error: {err:?}");
            return Ok(());
        }
    };

    if let Some(from) = cli.from {
        app.from = Some(from);
    }
    if let Some(from_name) = cli.from_name {
        app.from_name = Some(from_name);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = run(&mut terminal, app, client);

    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = terminal.show_cursor();

    if let Err(err) = result {
        eprintln!("Error: {err:?}");
    }

    Ok(())
}

// ── Startup ──────────────────────────────────────────────────────────────────

/// Loads an account config from disk when one exists at the standard
/// paths (or `$HIMALAYA_CONFIG`), otherwise runs the wizard to build
/// one in memory. The wizard never writes to disk; users who want to
/// skip it should create their own config file.
///
/// `account_or_seed` carries the CLI positional. When a config is
/// found, it is matched against the `[accounts]` table; otherwise it
/// is fed to the wizard as an email/URL/domain seed. When `no_config`
/// is set, the on-disk lookup is bypassed entirely and the wizard
/// runs unconditionally. `from` (the CLI `--from` flag) is forwarded
/// to the wizard to prefill SASL/JMAP login prompts.
fn load_then_connect(
    config_paths: &[PathBuf],
    account_or_seed: Option<&str>,
    no_config: bool,
    from: Option<&str>,
    keybinds_cli: Option<Keybinds>,
) -> Result<(App, EmailClientStd)> {
    let loaded = if no_config {
        None
    } else {
        Config::from_paths_or_default(config_paths)?
    };

    let (name, mut account_config, display_name, signature, keybinds_config) = match loaded {
        Some(mut config) => {
            let display = config.display_name.take();
            let sig = config.signature.take().unwrap_or_default();
            let keybinds = config.keybinds;
            let (name, account) = config
                .take_account(account_or_seed)?
                .ok_or_else(|| anyhow!("Account not found"))?;
            (name, account, display, sig, keybinds)
        }
        None => {
            let account = run_wizard(account_or_seed, from)?;
            ("default".to_string(), account, None, String::new(), None)
        }
    };

    let from = account_config.from.clone();
    let from_name = account_config.from_name.take().or(display_name);
    let signature = account_config.signature.take().unwrap_or(signature);
    let smtp_config = account_config.smtp.clone();

    // CLI > config; `None` keeps the global translation layer off.
    let keybinds = keybinds_cli.or(keybinds_config);

    let client = build_client(account_config)?;

    let app = App::new(name, from, from_name, signature, smtp_config, keybinds);
    Ok((app, client))
}

#[cfg(all(feature = "imap", feature = "smtp", feature = "jmap"))]
fn run_wizard(seed: Option<&str>, from: Option<&str>) -> Result<AccountConfig> {
    match seed {
        Some(seed) => wizard::discover::run_with_input(seed, from),
        None => wizard::discover::run(from),
    }
}

#[cfg(not(all(feature = "imap", feature = "smtp", feature = "jmap")))]
fn run_wizard(_seed: Option<&str>, _from: Option<&str>) -> Result<AccountConfig> {
    Err(anyhow!(
        "No config found and the wizard requires imap+smtp+jmap features."
    ))
}

/// Registers each configured backend on the unified client. Order is
/// JMAP → IMAP → Maildir for storage (richest first), then SMTP last
/// so JMAP-only accounts keep sending via JMAP and IMAP/Maildir
/// accounts pick up SMTP for sending.
#[allow(unused_variables, unused_mut)]
fn build_client(account_config: AccountConfig) -> Result<EmailClientStd> {
    let mut client = EmailClientStd::new();
    let mut configured = false;

    #[cfg(feature = "jmap")]
    if let Some(jmap_cfg) = account_config.jmap {
        client = client.with_jmap(jmap_cfg.into_client()?);
        configured = true;
    }

    #[cfg(feature = "imap")]
    if let Some(imap_cfg) = account_config.imap {
        client = client.with_imap(imap_cfg.into_client()?);
        configured = true;
    }

    #[cfg(feature = "maildir")]
    if let Some(maildir_cfg) = account_config.maildir {
        client = client.with_maildir(maildir_cfg.into_client());
        configured = true;
    }

    #[cfg(feature = "smtp")]
    if let Some(smtp_cfg) = account_config.smtp {
        match smtp_cfg.into_client() {
            Ok(smtp) => client = client.with_smtp(smtp),
            Err(err) => log::warn!("SMTP backend disabled: {err}. Sending will be unavailable."),
        }
    }

    if !configured {
        anyhow::bail!("Wizard produced no usable backend");
    }

    Ok(client)
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut app: App,
    mut client: EmailClientStd,
) -> Result<()> {
    app.set_status("Connecting...");
    terminal.draw(|f| ui::render(f, &mut app))?;

    match client.list_mailboxes(false) {
        Ok(mailboxes) => {
            app.set_mailboxes(mailboxes);
            load_envelopes(&mut app, &mut client);
        }
        Err(e) => app.set_status(format!("Error: {e}")),
    }

    run_app(terminal, &mut app, &mut client)
}

// ── Backend operations ───────────────────────────────────────────────────────

fn load_envelopes(app: &mut App, client: &mut EmailClientStd) {
    let Some(mailbox) = app.selected_mailbox.clone() else {
        return;
    };

    let page = Some(app.envelope_page as u32 + 1);
    let page_size = Some(app.envelope_page_size as u32);

    match client.list_envelopes(&mailbox, page, page_size, false) {
        Ok(envelopes) => {
            // Total isn't returned by the shared API; approximate with
            // the current page length for now.
            let total = envelopes.len() as u32;
            app.set_envelopes(envelopes, total);
        }
        Err(e) => app.set_status(format!("Error: {e}")),
    }
}

// ── Event loop ───────────────────────────────────────────────────────────────

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(60);

/// Maps mode-specific navigation shortcuts onto the universal keys
/// (`Up`, `Down`, `PageUp`, `PageDown`, `Esc`) consumed by the dialog
/// and global event branches. Returns the original event when no
/// flavor was configured or when no translation applies.
///
/// Emacs flavor: `Ctrl-n`/`Ctrl-p` (line nav), `Ctrl-v`/`Alt-v`
/// (page nav), `Ctrl-g` (cancel).
///
/// Vim flavor: `j`/`k` (line nav), `Ctrl-d`/`Ctrl-u` (page nav), `q`
/// (cancel).
fn translate_key(key: event::KeyEvent, kb: Option<Keybinds>) -> event::KeyEvent {
    let Some(kb) = kb else { return key };
    let modifiers = key.modifiers;
    let translated = match kb {
        Keybinds::Emacs if modifiers == KeyModifiers::CONTROL => match key.code {
            KeyCode::Char('n') => Some(KeyCode::Down),
            KeyCode::Char('p') => Some(KeyCode::Up),
            KeyCode::Char('v') => Some(KeyCode::PageDown),
            KeyCode::Char('g') => Some(KeyCode::Esc),
            _ => None,
        },
        Keybinds::Emacs if modifiers == KeyModifiers::ALT => match key.code {
            KeyCode::Char('v') => Some(KeyCode::PageUp),
            _ => None,
        },
        Keybinds::Vim if modifiers == KeyModifiers::NONE => match key.code {
            KeyCode::Char('j') => Some(KeyCode::Down),
            KeyCode::Char('k') => Some(KeyCode::Up),
            KeyCode::Char('q') => Some(KeyCode::Esc),
            _ => None,
        },
        Keybinds::Vim if modifiers == KeyModifiers::CONTROL => match key.code {
            KeyCode::Char('d') => Some(KeyCode::PageDown),
            KeyCode::Char('u') => Some(KeyCode::PageUp),
            _ => None,
        },
        _ => None,
    };

    match translated {
        Some(code) => event::KeyEvent::new(code, KeyModifiers::NONE),
        None => key,
    }
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    client: &mut EmailClientStd,
) -> Result<()> {
    while app.running {
        if app.active_panel == Panel::Compose && system_editor::is_pending(&app.editor_state) {
            system_editor::open(&mut app.editor_state, terminal)?;
            execute!(terminal.backend_mut(), EnableMouseCapture)?;
        }

        terminal.draw(|f| ui::render(f, app))?;

        if !event::poll(KEEPALIVE_INTERVAL)? {
            continue;
        }

        if let Event::Key(raw_key) = event::read()? {
            if raw_key.kind != KeyEventKind::Press {
                continue;
            }

            // The composer hands raw keys to edtui (which already knows
            // Vim vs Emacs); every other branch goes through the
            // translation layer so Ctrl-n/Ctrl-p (Emacs) or j/k (Vim)
            // alias the universal arrow/page keys.
            let in_composer = app.dialog.is_none() && app.active_panel == Panel::Compose;
            let key = if in_composer {
                raw_key
            } else {
                translate_key(raw_key, app.keybinds)
            };

            if let Some(dialog) = app.dialog {
                match dialog {
                    Dialog::Envelope => handle_envelope_dialog(app, key.code, client),
                    Dialog::Compose => handle_compose_dialog(app, key.code, client),
                    Dialog::CopyTo => handle_copy_to_dialog(app, key.code, client),
                    Dialog::MoveTo => handle_move_to_dialog(app, key.code, client),
                    Dialog::FlagAdd => handle_flag_dialog(app, key.code, client, true),
                    Dialog::FlagRemove => handle_flag_dialog(app, key.code, client, false),
                }
                continue;
            }

            if in_composer {
                if key.code == KeyCode::Esc {
                    app.open_dialog(Dialog::Compose);
                    continue;
                }

                if key.code == KeyCode::Char('e') && key.modifiers.contains(KeyModifiers::ALT) {
                    OpenSystemEditor.execute(&mut app.editor_state);
                } else {
                    app.editor_handler.on_key_event(key, &mut app.editor_state);
                }

                if system_editor::is_pending(&app.editor_state) {
                    system_editor::open(&mut app.editor_state, terminal)?;
                    execute!(terminal.backend_mut(), EnableMouseCapture)?;
                }

                continue;
            }

            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.start_compose();
                continue;
            }

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
                KeyCode::Enter => handle_enter(app, client),
                KeyCode::PageDown => {
                    if app.active_panel == Panel::Envelopes && app.next_envelope_page() {
                        load_envelopes(app, client);
                    }
                }
                KeyCode::PageUp => {
                    if app.active_panel == Panel::Envelopes && app.prev_envelope_page() {
                        load_envelopes(app, client);
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

// ── Dialog and key handlers ──────────────────────────────────────────────────

fn handle_enter(app: &mut App, client: &mut EmailClientStd) {
    match app.active_panel {
        Panel::Mailboxes => {
            app.select_mailbox();
            load_envelopes(app, client);
        }
        Panel::Envelopes => {
            if app.get_selected_envelope().is_some() {
                app.open_dialog(Dialog::Envelope);
            }
        }
        Panel::Message => app.close_bottom_panel(),
        Panel::Compose => {}
    }
}

fn handle_envelope_dialog(app: &mut App, key: KeyCode, client: &mut EmailClientStd) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let action = app.get_selected_envelope_action();
            app.close_dialog();
            match action {
                EnvelopeAction::Read => handle_read_message(app, client),
                EnvelopeAction::Reply => handle_reply(app, client, false),
                EnvelopeAction::ReplyAll => handle_reply(app, client, true),
                EnvelopeAction::Forward => handle_forward(app, client),
                EnvelopeAction::Copy => app.open_dialog(Dialog::CopyTo),
                EnvelopeAction::Move => app.open_dialog(Dialog::MoveTo),
                EnvelopeAction::AddFlag => app.open_dialog(Dialog::FlagAdd),
                EnvelopeAction::RemoveFlag => app.open_dialog(Dialog::FlagRemove),
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

fn handle_read_message(app: &mut App, client: &mut EmailClientStd) {
    let Some(envelope) = app.get_selected_envelope().cloned() else {
        return;
    };
    let Some(mailbox) = app.selected_mailbox.clone() else {
        return;
    };
    app.set_status(format!("Loading message {}...", envelope.id));

    match client.get_message(&mailbox, &envelope.id) {
        Ok(raw) => match decode_message_body(&raw) {
            Ok(content) => app.show_message(content),
            Err(e) => app.set_status(format!("Error: {e}")),
        },
        Err(e) => app.set_status(format!("Error: {e}")),
    }
}

fn handle_reply(app: &mut App, client: &mut EmailClientStd, reply_all: bool) {
    let Some(envelope) = app.get_selected_envelope().cloned() else {
        return;
    };
    let Some(mailbox) = app.selected_mailbox.clone() else {
        return;
    };
    app.set_status(format!("Loading message {}...", envelope.id));

    match client.get_message(&mailbox, &envelope.id) {
        Ok(raw) => app.start_reply(&raw, reply_all),
        Err(e) => app.set_status(format!("Error: {e}")),
    }
}

fn handle_forward(app: &mut App, client: &mut EmailClientStd) {
    let Some(envelope) = app.get_selected_envelope().cloned() else {
        return;
    };
    let Some(mailbox) = app.selected_mailbox.clone() else {
        return;
    };
    app.set_status(format!("Loading message {}...", envelope.id));

    match client.get_message(&mailbox, &envelope.id) {
        Ok(raw) => app.start_forward(&raw),
        Err(e) => app.set_status(format!("Error: {e}")),
    }
}

fn handle_copy_to_dialog(app: &mut App, key: KeyCode, client: &mut EmailClientStd) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let target = app.mailboxes.get(app.dialog_index).cloned();
            app.close_dialog();

            let Some(target) = target else { return };
            let Some(envelope) = app.get_selected_envelope().cloned() else {
                return;
            };
            let Some(mailbox) = app.selected_mailbox.clone() else {
                return;
            };

            app.set_status(format!("Copying to {}...", target.name));
            match client.copy_messages(&mailbox, &target.id, &[&envelope.id]) {
                Ok(()) => app.set_status("Copied"),
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

fn handle_move_to_dialog(app: &mut App, key: KeyCode, client: &mut EmailClientStd) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let target = app.mailboxes.get(app.dialog_index).cloned();
            app.close_dialog();

            let Some(target) = target else { return };
            let Some(envelope) = app.get_selected_envelope().cloned() else {
                return;
            };
            let Some(mailbox) = app.selected_mailbox.clone() else {
                return;
            };

            app.set_status(format!("Moving to {}...", target.name));
            match client.move_messages(&mailbox, &target.id, &[&envelope.id]) {
                Ok(()) => {
                    app.remove_selected_envelope();
                    app.set_status("Moved");
                }
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

fn handle_flag_dialog(app: &mut App, key: KeyCode, client: &mut EmailClientStd, add: bool) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let action = app.get_selected_flag_action();
            app.close_dialog();

            let Some(envelope) = app.get_selected_envelope().cloned() else {
                return;
            };
            let Some(mailbox) = app.selected_mailbox.clone() else {
                return;
            };

            let flag = action.flag();
            let label = action.label();
            let verb = if add { "Adding" } else { "Removing" };
            app.set_status(format!("{verb} flag {label}..."));

            let result = if add {
                client.add_flags(&mailbox, &[&envelope.id], &[flag])
            } else {
                client.delete_flags(&mailbox, &[&envelope.id], &[flag])
            };

            match result {
                Ok(()) if add => {
                    app.flag_selected_envelope(flag);
                    app.set_status(format!("Flag {label} added"));
                }
                Ok(()) => {
                    app.unflag_selected_envelope(flag);
                    app.set_status(format!("Flag {label} removed"));
                }
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

fn handle_compose_dialog(app: &mut App, key: KeyCode, client: &mut EmailClientStd) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let action = app.get_selected_compose_action();
            match action {
                ComposeAction::Send => {
                    let content = app.get_compose_content();
                    app.set_status("Compiling message...");
                    match MmlCompilerBuilder::new().build(&content) {
                        Ok(compiler) => match compiler.compile() {
                            Ok(result) => match result.into_vec() {
                                Ok(mime_bytes) => send_compiled(app, mime_bytes, client),
                                Err(e) => app.set_status(format!("Error: {e}")),
                            },
                            Err(e) => app.set_status(format!("Compile error: {e}")),
                        },
                        Err(e) => app.set_status(format!("Parse error: {e}")),
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
                            Err(e) => app.set_status(format!("Compile error: {e}")),
                        },
                        Err(e) => app.set_status(format!("Parse error: {e}")),
                    }
                }
                ComposeAction::SaveToDrafts => save_to_drafts(app, client),
                ComposeAction::Cancel => app.close_dialog(),
            }
        }
        KeyCode::Esc => app.cancel_compose(),
        _ => {}
    }
}

fn save_to_drafts(app: &mut App, client: &mut EmailClientStd) {
    let content = app.get_compose_content();
    let raw = format!(
        "From: \r\nTo: \r\nSubject: Draft\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{content}"
    )
    .into_bytes();

    app.set_status("Saving to Drafts...");

    match client.add_message("Drafts", &[Flag::Draft], raw) {
        Ok(_) => {
            app.set_status("Saved to Drafts");
            app.cancel_compose();
        }
        Err(e) => app.set_status(format!("Error: {e}")),
    }
}

fn send_compiled(app: &mut App, mime_bytes: Vec<u8>, client: &mut EmailClientStd) {
    let (from, to) = match extract_envelope(&mime_bytes) {
        Ok(env) => env,
        Err(e) => {
            app.set_status(format!("Send error: {e}"));
            return;
        }
    };
    let to_refs: Vec<&str> = to.iter().map(String::as_str).collect();

    app.set_status("Sending message...");
    match client.send_message(mime_bytes, &from, &to_refs) {
        Ok(()) => {
            app.set_status("Message sent");
            app.cancel_compose();
        }
        Err(e) => app.set_status(format!("Send error: {e}")),
    }
}

// ── MIME helpers ─────────────────────────────────────────────────────────────

fn decode_message_body(raw: &[u8]) -> Result<String> {
    let message = mail_parser::MessageParser::default()
        .parse(raw)
        .ok_or_else(|| anyhow!("Failed to parse message"))?;

    if let Some(text) = message.body_text(0) {
        Ok(text.to_string())
    } else if let Some(html) = message.body_html(0) {
        Ok(html2text::from_read(html.as_bytes(), 80)?)
    } else {
        Ok(String::from_utf8_lossy(raw).to_string())
    }
}

/// Extracts the envelope sender and recipients from the raw RFC 5322
/// message headers. Used for SMTP routing on send.
fn extract_envelope(raw: &[u8]) -> Result<(String, Vec<String>)> {
    use std::collections::HashSet;

    use mail_parser::{Address, HeaderName, HeaderValue, MessageParser};

    let msg = MessageParser::new()
        .parse_headers(raw)
        .ok_or_else(|| anyhow!("Invalid message to send"))?;

    let mut mail_from: Option<String> = None;
    let mut rcpt_to: HashSet<String> = HashSet::new();

    for header in msg.headers() {
        let key = &header.name;
        let val = header.value();

        match key {
            HeaderName::From => {
                if let HeaderValue::Address(Address::List(addrs)) = val {
                    if let Some(email) = addrs.first().and_then(valid_email) {
                        mail_from = Some(email);
                    }
                } else if let HeaderValue::Address(Address::Group(groups)) = val {
                    if let Some(group) = groups.first() {
                        if let Some(email) = group.addresses.first().and_then(valid_email) {
                            mail_from = Some(email);
                        }
                    }
                }
            }
            HeaderName::To | HeaderName::Cc | HeaderName::Bcc => match val {
                HeaderValue::Address(Address::List(addrs)) => {
                    rcpt_to.extend(addrs.iter().filter_map(valid_email));
                }
                HeaderValue::Address(Address::Group(groups)) => {
                    rcpt_to.extend(
                        groups
                            .iter()
                            .flat_map(|group| group.addresses.iter())
                            .filter_map(valid_email),
                    );
                }
                _ => (),
            },
            _ => (),
        };
    }

    let mail_from = mail_from.ok_or_else(|| anyhow!("The message does not contain any sender"))?;
    if rcpt_to.is_empty() {
        anyhow::bail!("The message does not contain any recipient");
    }

    Ok((mail_from, rcpt_to.into_iter().collect()))
}

fn valid_email(addr: &mail_parser::Addr) -> Option<String> {
    addr.address.as_ref().and_then(|email| {
        let email = email.trim();
        if email.is_empty() {
            None
        } else {
            Some(email.to_string())
        }
    })
}

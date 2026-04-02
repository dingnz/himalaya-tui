use std::{fs::File, io, path::PathBuf, time::Duration};

use anyhow::Result;
use edtui::{
    actions::{system_editor, Execute, OpenSystemEditor},
    EditorEventHandler,
};
#[cfg(feature = "jmap")]
use himalaya_tui::jmap::*;
#[cfg(feature = "smtp")]
use himalaya_tui::smtp::SmtpMessageSendHandler;
#[cfg(feature = "imap")]
use himalaya_tui::{app::FlagAction, imap::*};
use himalaya_tui::{
    app::{App, ComposeAction, Dialog, EnvelopeAction, Panel},
    config::{AccountConfig, Config, SmtpConfig},
    ui,
};
#[cfg(feature = "imap")]
use io_imap::{
    coroutines::noop::{ImapNoop, ImapNoopResult},
    types::flag::Flag,
};
#[cfg(feature = "jmap")]
use io_jmap::rfc8620::coroutines::session_get::{JmapSessionGet, JmapSessionGetResult};
use io_stream::runtimes::std::handle;
use mml::message::compiler::MmlCompilerBuilder;
use pimalaya_toolbox::config::TomlConfig;
#[cfg(feature = "jmap")]
use pimalaya_toolbox::stream::jmap::{JmapAuth, JmapSession};
use pimalaya_toolbox::stream::Tls;
#[cfg(feature = "imap")]
use pimalaya_toolbox::{
    sasl::{sasl_default_mechanisms, Sasl, SaslLogin, SaslPlain},
    stream::imap::ImapSession,
};
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
use secrecy::SecretString;
use url::Url;

enum Backend {
    #[cfg(feature = "imap")]
    Imap(ImapSession),
    #[cfg(feature = "jmap")]
    Jmap(JmapSession),
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let log_file = File::create("/tmp/himalaya-tui.log")?;
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Trace,
        simplelog::Config::default(),
        log_file,
    )?;

    let config_paths = get_config_paths();
    let account_name = std::env::args().nth(1);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = run(&mut terminal, &config_paths, account_name.as_deref());

    // Always restore the terminal, even on error.
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

// ── Startup ──────────────────────────────────────────────────────────────────

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config_paths: &[PathBuf],
    account_name: Option<&str>,
) -> Result<()> {
    let (mut app, mut backend) = match try_from_config(config_paths, account_name) {
        Ok((name, email, display_name, signature, smtp_config, backend)) => (
            App::new(name, email, display_name, signature, smtp_config),
            backend,
        ),
        Err(_) => {
            let mut app = App::new_wizard();
            let backend = run_wizard_tui(terminal, &mut app)?;
            (app, backend)
        }
    };

    app.set_status("Connecting...");
    terminal.draw(|f| ui::render(f, &mut app))?;

    match load_mailboxes(&mut backend) {
        Ok(mailboxes) => {
            app.set_mailboxes(mailboxes);
            load_envelopes(&mut app, &mut backend);
        }
        Err(e) => app.set_status(format!("Error: {e}")),
    }

    run_app(terminal, &mut app, &mut backend)
}

type StartupTuple = (String, String, String, String, Option<SmtpConfig>, Backend);

fn try_from_config(config_paths: &[PathBuf], account_name: Option<&str>) -> Result<StartupTuple> {
    let config = Config::from_paths_or_default(config_paths)?;
    let (name, account_config) = config.get_account(account_name)?;
    let email = account_config.email.clone();
    let display_name = account_config
        .display_name
        .clone()
        .or_else(|| config.display_name.clone())
        .unwrap_or_default();
    let signature = account_config
        .signature
        .clone()
        .or_else(|| config.signature.clone())
        .unwrap_or_default();
    let smtp_config = account_config.smtp.clone();
    let backend = build_backend(&name, account_config)?;
    Ok((name, email, display_name, signature, smtp_config, backend))
}

fn build_backend(name: &str, account_config: AccountConfig) -> Result<Backend> {
    #[cfg(feature = "jmap")]
    if let Some(jmap_cfg) = account_config.jmap {
        return Ok(Backend::Jmap(jmap_cfg.into_session()?));
    }

    #[cfg(feature = "imap")]
    if let Some(imap_cfg) = account_config.imap {
        return Ok(Backend::Imap(imap_cfg.into_session()?));
    }

    anyhow::bail!("No supported backend configured for account `{name}`")
}

fn run_wizard_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<Backend> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            let wizard = app.wizard.as_mut().expect("wizard state");

            match key.code {
                KeyCode::Esc => anyhow::bail!("Setup cancelled"),
                KeyCode::Tab => wizard.next_field(),
                KeyCode::BackTab => wizard.prev_field(),
                KeyCode::Backspace => wizard.backspace(),
                KeyCode::Char(c) => wizard.push_char(c),
                KeyCode::Enter => {
                    let uri = wizard.uri.clone();
                    let username = wizard.username.clone();
                    let password = wizard.password.clone();
                    wizard.connecting = true;
                    wizard.error = None;

                    terminal.draw(|f| ui::render(f, app))?;

                    match try_connect(&uri, &username, &password) {
                        Ok((name, email, backend)) => {
                            app.complete_wizard(name, email);
                            return Ok(backend);
                        }
                        Err(e) => {
                            let wizard = app.wizard.as_mut().expect("wizard state");
                            wizard.connecting = false;
                            wizard.error = Some(e.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn try_connect(
    uri_str: &str,
    username_arg: &str,
    password: &str,
) -> Result<(String, String, Backend)> {
    let url = Url::parse(uri_str.trim())?;
    let scheme = url.scheme().to_lowercase();
    let host = url.host_str().unwrap_or("").to_string();

    let username_from_uri = url.username().to_string();
    let username = if !username_from_uri.is_empty() {
        username_from_uri
    } else {
        username_arg.to_string()
    };

    let password: SecretString = password.to_string().into();

    let email = if username.contains('@') {
        username.clone()
    } else {
        format!("{username}@{host}")
    };

    let name = username.split('@').next().unwrap_or(&username).to_string();

    let backend = match scheme.as_str() {
        #[cfg(feature = "imap")]
        "imap" | "imaps" => {
            let sasl = Sasl {
                mechanisms: sasl_default_mechanisms(),
                plain: Some(SaslPlain {
                    authzid: None,
                    authcid: username.clone(),
                    passwd: password.clone(),
                }),
                login: Some(SaslLogin {
                    username: username.clone(),
                    password: password.clone(),
                }),
                anonymous: None,
            };
            Backend::Imap(ImapSession::new(url, Tls::default(), false, sasl)?)
        }
        #[cfg(feature = "jmap")]
        "jmap" | "jmaps" | "http" | "https" => {
            let auth = JmapAuth::Basic { username, password };
            Backend::Jmap(JmapSession::new(url.to_string(), Tls::default(), auth)?)
        }
        _ => anyhow::bail!(
            "Unsupported URI scheme `{scheme}`. Use imap://, imaps://, https://, or http://"
        ),
    };

    Ok((name, email, backend))
}

// ── Backend operations ───────────────────────────────────────────────────────

fn load_mailboxes(backend: &mut Backend) -> Result<Vec<himalaya_tui::app::Mailbox>> {
    match backend {
        #[cfg(feature = "imap")]
        Backend::Imap(session) => ImapMailboxListHandler.execute(session),
        #[cfg(feature = "jmap")]
        Backend::Jmap(session) => JmapMailboxListHandler.execute(session),
    }
}

fn load_envelopes(app: &mut App, backend: &mut Backend) {
    match backend {
        #[cfg(feature = "imap")]
        Backend::Imap(session) => {
            let Some(mailbox) = app.selected_mailbox.clone() else {
                return;
            };
            match (ImapEnvelopeListHandler {
                mailbox,
                page: app.envelope_page,
                page_size: app.envelope_page_size,
            })
            .execute(session)
            {
                Ok((envelopes, total)) => app.set_envelopes(envelopes, total),
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
        #[cfg(feature = "jmap")]
        Backend::Jmap(session) => {
            let Some(mailbox_id) = app.selected_mailbox_id.clone() else {
                app.set_status("No mailbox ID available");
                return;
            };
            match (JmapEnvelopeListHandler {
                mailbox_id,
                page: app.envelope_page,
                page_size: app.envelope_page_size,
            })
            .execute(session)
            {
                Ok((envelopes, total)) => app.set_envelopes(envelopes, total),
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
    }
}

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(60);

fn send_keepalive(backend: &mut Backend) {
    match backend {
        #[cfg(feature = "imap")]
        Backend::Imap(session) => {
            let context = std::mem::take(&mut session.context);
            let mut coroutine = ImapNoop::new(context);
            let mut arg = None;
            loop {
                match coroutine.resume(arg.take()) {
                    ImapNoopResult::Io { io } => match handle(&mut session.stream, io) {
                        Ok(io) => arg = Some(io),
                        Err(_) => break,
                    },
                    ImapNoopResult::Ok { context } => {
                        session.context = context;
                        break;
                    }
                    ImapNoopResult::Err { context, .. } => {
                        session.context = context;
                        break;
                    }
                }
            }
        }
        #[cfg(feature = "jmap")]
        Backend::Jmap(session) => {
            let Ok(url) = session.session.api_url.as_str().parse() else {
                return;
            };
            let Ok(mut coroutine) = JmapSessionGet::new(&session.http_auth, &url) else {
                return;
            };
            let mut arg = None;
            loop {
                match coroutine.resume(arg.take()) {
                    JmapSessionGetResult::Io { io } => match handle(&mut session.stream, io) {
                        Ok(io) => arg = Some(io),
                        Err(_) => break,
                    },
                    JmapSessionGetResult::Ok {
                        session: new_session,
                        ..
                    } => {
                        session.session = new_session;
                        break;
                    }
                    JmapSessionGetResult::Reset(_) | JmapSessionGetResult::Err { .. } => break,
                }
            }
        }
    }
}

// ── Event loop ───────────────────────────────────────────────────────────────

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    backend: &mut Backend,
) -> Result<()> {
    let mut editor_handler = EditorEventHandler::default();

    while app.running {
        if app.active_panel == Panel::Compose && system_editor::is_pending(&app.editor_state) {
            system_editor::open(&mut app.editor_state, terminal)?;
            execute!(terminal.backend_mut(), EnableMouseCapture)?;
        }

        terminal.draw(|f| ui::render(f, app))?;

        if !event::poll(KEEPALIVE_INTERVAL)? {
            send_keepalive(backend);
            continue;
        }

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if let Some(dialog) = app.dialog {
                match dialog {
                    Dialog::Envelope => handle_envelope_dialog(app, key.code, backend),
                    Dialog::Compose => handle_compose_dialog(app, key.code, backend),
                    Dialog::CopyTo => handle_copy_to_dialog(app, key.code, backend),
                    Dialog::MoveTo => handle_move_to_dialog(app, key.code, backend),
                    Dialog::Delete => handle_delete_dialog(app, key.code, backend),
                    Dialog::FlagAdd => handle_flag_dialog(app, key.code, backend, true),
                    Dialog::FlagRemove => handle_flag_dialog(app, key.code, backend, false),
                }
                continue;
            }

            if app.active_panel == Panel::Compose {
                if key.code == KeyCode::Esc {
                    app.open_dialog(Dialog::Compose);
                    continue;
                }

                if key.code == KeyCode::Char('e') && key.modifiers.contains(KeyModifiers::ALT) {
                    OpenSystemEditor.execute(&mut app.editor_state);
                } else {
                    editor_handler.on_key_event(key, &mut app.editor_state);
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
                KeyCode::Enter => handle_enter(app, backend),
                KeyCode::PageDown => {
                    if app.active_panel == Panel::Envelopes && app.next_envelope_page() {
                        load_envelopes(app, backend);
                    }
                }
                KeyCode::PageUp => {
                    if app.active_panel == Panel::Envelopes && app.prev_envelope_page() {
                        load_envelopes(app, backend);
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

// ── Dialog and key handlers ──────────────────────────────────────────────────

fn handle_enter(app: &mut App, backend: &mut Backend) {
    match app.active_panel {
        Panel::Mailboxes => {
            app.select_mailbox();
            load_envelopes(app, backend);
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

fn handle_envelope_dialog(app: &mut App, key: KeyCode, backend: &mut Backend) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let action = app.get_selected_envelope_action();
            app.close_dialog();
            match action {
                EnvelopeAction::Read => handle_read_message(app, backend),
                EnvelopeAction::Reply => handle_reply(app, backend, false),
                EnvelopeAction::ReplyAll => handle_reply(app, backend, true),
                EnvelopeAction::Forward => handle_forward(app, backend),
                EnvelopeAction::Copy => app.open_dialog(Dialog::CopyTo),
                EnvelopeAction::Move => app.open_dialog(Dialog::MoveTo),
                EnvelopeAction::Delete => app.open_dialog(Dialog::Delete),
                EnvelopeAction::AddFlag => app.open_dialog(Dialog::FlagAdd),
                EnvelopeAction::RemoveFlag => app.open_dialog(Dialog::FlagRemove),
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

fn handle_read_message(app: &mut App, backend: &mut Backend) {
    let Some(envelope) = app.get_selected_envelope().cloned() else {
        return;
    };
    app.set_status(format!("Loading message {}...", envelope.id));

    match backend {
        #[cfg(feature = "imap")]
        Backend::Imap(session) => {
            let Some(mailbox) = app.selected_mailbox.clone() else {
                return;
            };
            match (ImapMessageGetHandler {
                mailbox,
                id: envelope.id,
            })
            .execute(session)
            {
                Ok(content) => app.show_message(content),
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
        #[cfg(feature = "jmap")]
        Backend::Jmap(session) => {
            match (JmapMessageGetHandler { id: envelope.id }).execute(session) {
                Ok(content) => app.show_message(content),
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
    }
}

fn handle_reply(app: &mut App, backend: &mut Backend, reply_all: bool) {
    let Some(envelope) = app.get_selected_envelope().cloned() else {
        return;
    };
    app.set_status(format!("Loading message {}...", envelope.id));

    match backend {
        #[cfg(feature = "imap")]
        Backend::Imap(session) => {
            let Some(mailbox) = app.selected_mailbox.clone() else {
                return;
            };
            match (ImapMessageGetRawHandler {
                mailbox,
                id: envelope.id,
            })
            .execute(session)
            {
                Ok(raw) => {
                    app.start_reply(&raw, reply_all);
                }
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
        #[cfg(feature = "jmap")]
        Backend::Jmap(session) => {
            match (JmapMessageGetRawHandler { id: envelope.id }).execute(session) {
                Ok(raw) => {
                    app.start_reply(&raw, reply_all);
                }
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
    }
}

fn handle_forward(app: &mut App, backend: &mut Backend) {
    let Some(envelope) = app.get_selected_envelope().cloned() else {
        return;
    };
    app.set_status(format!("Loading message {}...", envelope.id));

    match backend {
        #[cfg(feature = "imap")]
        Backend::Imap(session) => {
            let Some(mailbox) = app.selected_mailbox.clone() else {
                return;
            };
            match (ImapMessageGetRawHandler {
                mailbox,
                id: envelope.id,
            })
            .execute(session)
            {
                Ok(raw) => {
                    app.start_forward(&raw);
                }
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
        #[cfg(feature = "jmap")]
        Backend::Jmap(session) => {
            match (JmapMessageGetRawHandler { id: envelope.id }).execute(session) {
                Ok(raw) => {
                    app.start_forward(&raw);
                }
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
    }
}

fn handle_delete_dialog(app: &mut App, key: KeyCode, backend: &mut Backend) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let confirmed = app.dialog_index == 0;
            app.close_dialog();

            if !confirmed {
                return;
            }

            let Some(envelope) = app.get_selected_envelope().cloned() else {
                return;
            };
            app.set_status(format!("Deleting message {}...", envelope.id));

            match backend {
                #[cfg(feature = "imap")]
                Backend::Imap(session) => {
                    let Some(mailbox) = app.selected_mailbox.clone() else {
                        return;
                    };
                    match (ImapMessageDeleteHandler {
                        mailbox,
                        id: envelope.id,
                    })
                    .execute(session)
                    {
                        Ok(_) => {
                            app.flag_selected_envelope("\\Deleted");
                            app.set_status("Message flagged as deleted");
                        }
                        Err(e) => app.set_status(format!("Error: {e}")),
                    }
                }
                #[cfg(feature = "jmap")]
                Backend::Jmap(session) => {
                    match (JmapMessageDeleteHandler { id: envelope.id }).execute(session) {
                        Ok(_) => {
                            app.remove_selected_envelope();
                            app.set_status("Message deleted");
                        }
                        Err(e) => app.set_status(format!("Error: {e}")),
                    }
                }
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

fn handle_copy_to_dialog(app: &mut App, key: KeyCode, backend: &mut Backend) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let target_mailbox = app.mailboxes.get(app.dialog_index).cloned();
            app.close_dialog();

            let Some(target_mailbox) = target_mailbox else {
                return;
            };
            let Some(envelope) = app.get_selected_envelope().cloned() else {
                return;
            };

            match backend {
                #[cfg(feature = "imap")]
                Backend::Imap(session) => {
                    let Some(mailbox) = app.selected_mailbox.clone() else {
                        return;
                    };
                    let target = target_mailbox.name;
                    app.set_status(format!("Copying to {target}..."));
                    match (ImapMessageCopyHandler {
                        mailbox,
                        id: envelope.id,
                        target,
                    })
                    .execute(session)
                    {
                        Ok(_) => app.set_status("Copied"),
                        Err(e) => app.set_status(format!("Error: {e}")),
                    }
                }
                #[cfg(feature = "jmap")]
                Backend::Jmap(session) => {
                    let Some(target_mailbox_id) = target_mailbox.id else {
                        app.set_status("Target mailbox has no ID");
                        return;
                    };
                    app.set_status(format!("Copying to {}...", target_mailbox.name));
                    match (JmapMessageCopyHandler {
                        id: envelope.id,
                        target_mailbox_id,
                    })
                    .execute(session)
                    {
                        Ok(_) => app.set_status("Copied"),
                        Err(e) => app.set_status(format!("Error: {e}")),
                    }
                }
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

fn handle_move_to_dialog(app: &mut App, key: KeyCode, backend: &mut Backend) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let target_mailbox = app.mailboxes.get(app.dialog_index).cloned();
            app.close_dialog();

            let Some(target_mailbox) = target_mailbox else {
                return;
            };
            let Some(envelope) = app.get_selected_envelope().cloned() else {
                return;
            };

            match backend {
                #[cfg(feature = "imap")]
                Backend::Imap(session) => {
                    let Some(mailbox) = app.selected_mailbox.clone() else {
                        return;
                    };
                    let target = target_mailbox.name;
                    app.set_status(format!("Moving to {target}..."));
                    match (ImapMessageMoveHandler {
                        mailbox,
                        id: envelope.id,
                        target,
                    })
                    .execute(session)
                    {
                        Ok(_) => {
                            app.remove_selected_envelope();
                            app.set_status("Moved");
                        }
                        Err(e) => app.set_status(format!("Error: {e}")),
                    }
                }
                #[cfg(feature = "jmap")]
                Backend::Jmap(session) => {
                    let Some(target_mailbox_id) = target_mailbox.id else {
                        app.set_status("Target mailbox has no ID");
                        return;
                    };
                    app.set_status(format!("Moving to {}...", target_mailbox.name));
                    match (JmapMessageMoveHandler {
                        id: envelope.id,
                        target_mailbox_id,
                    })
                    .execute(session)
                    {
                        Ok(_) => {
                            app.remove_selected_envelope();
                            app.set_status("Moved");
                        }
                        Err(e) => app.set_status(format!("Error: {e}")),
                    }
                }
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

fn handle_flag_dialog(app: &mut App, key: KeyCode, backend: &mut Backend, add: bool) {
    match key {
        KeyCode::Down => app.dialog_next(),
        KeyCode::Up => app.dialog_previous(),
        KeyCode::Enter => {
            let flag_action = app.get_selected_flag_action();
            app.close_dialog();

            let Some(envelope) = app.get_selected_envelope().cloned() else {
                return;
            };
            let verb = if add { "Adding" } else { "Removing" };

            match backend {
                #[cfg(feature = "imap")]
                Backend::Imap(session) => {
                    let Some(mailbox) = app.selected_mailbox.clone() else {
                        return;
                    };
                    let flag = match flag_action {
                        FlagAction::Seen => Flag::Seen,
                        FlagAction::Flagged => Flag::Flagged,
                        FlagAction::Answered => Flag::Answered,
                    };
                    let flag_label = flag_action.label();
                    app.set_status(format!("{verb} flag {flag_label}..."));
                    let result = if add {
                        ImapFlagAddHandler {
                            mailbox,
                            id: envelope.id,
                            flags: vec![flag],
                        }
                        .execute(session)
                    } else {
                        ImapFlagRemoveHandler {
                            mailbox,
                            id: envelope.id,
                            flags: vec![flag],
                        }
                        .execute(session)
                    };
                    match result {
                        Ok(_) if add => {
                            app.flag_selected_envelope(flag_label);
                            app.set_status(format!("Flag {flag_label} added"));
                        }
                        Ok(_) => {
                            app.unflag_selected_envelope(flag_label);
                            app.set_status(format!("Flag {flag_label} removed"));
                        }
                        Err(e) => app.set_status(format!("Error: {e}")),
                    }
                }
                #[cfg(feature = "jmap")]
                Backend::Jmap(session) => {
                    let keyword = flag_action.jmap_keyword().to_string();
                    app.set_status(format!("{verb} flag {keyword}..."));
                    let (add_kw, remove_kw) = if add {
                        (vec![keyword.clone()], vec![])
                    } else {
                        (vec![], vec![keyword.clone()])
                    };
                    match (JmapFlagUpdateHandler {
                        id: envelope.id,
                        add: add_kw,
                        remove: remove_kw,
                    })
                    .execute(session)
                    {
                        Ok(_) if add => {
                            app.flag_selected_envelope(&keyword);
                            app.set_status(format!("Flag {keyword} added"));
                        }
                        Ok(_) => {
                            app.unflag_selected_envelope(&keyword);
                            app.set_status(format!("Flag {keyword} removed"));
                        }
                        Err(e) => app.set_status(format!("Error: {e}")),
                    }
                }
            }
        }
        KeyCode::Esc => app.close_dialog(),
        _ => {}
    }
}

fn handle_compose_dialog(app: &mut App, key: KeyCode, backend: &mut Backend) {
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
                                Ok(mime_bytes) => send_compiled(app, mime_bytes, backend),
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
                ComposeAction::SaveToDrafts => save_to_drafts(app, backend),
                ComposeAction::Cancel => app.close_dialog(),
            }
        }
        KeyCode::Esc => app.cancel_compose(),
        _ => {}
    }
}

fn save_to_drafts(app: &mut App, backend: &mut Backend) {
    let content = app.get_compose_content();
    let raw = format!(
        "From: \r\nTo: \r\nSubject: Draft\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{content}"
    )
    .into_bytes();

    app.set_status("Saving to Drafts...");

    match backend {
        #[cfg(feature = "imap")]
        Backend::Imap(session) => {
            match (ImapMessageSaveHandler {
                mailbox: "Drafts".to_string(),
                raw,
                flags: vec![Flag::Draft],
            })
            .execute(session)
            {
                Ok(_) => {
                    app.set_status("Saved to Drafts");
                    app.cancel_compose();
                }
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
        #[cfg(feature = "jmap")]
        Backend::Jmap(session) => {
            let Some(mailbox_id) = app
                .mailboxes
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case("Drafts"))
                .and_then(|m| m.id.clone())
            else {
                app.set_status("No Drafts mailbox found");
                return;
            };
            match (JmapMessageSaveHandler { mailbox_id, raw }).execute(session) {
                Ok(_) => {
                    app.set_status("Saved to Drafts");
                    app.cancel_compose();
                }
                Err(e) => app.set_status(format!("Error: {e}")),
            }
        }
    }
}

fn send_compiled(app: &mut App, mime_bytes: Vec<u8>, backend: &mut Backend) {
    app.set_status("Sending message...");

    match backend {
        #[cfg(feature = "jmap")]
        Backend::Jmap(session) => {
            let sent_mailbox_id = app
                .mailboxes
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case("Sent"))
                .and_then(|m| m.id.clone());
            match (JmapMessageSendHandler {
                raw: mime_bytes,
                sent_mailbox_id,
                envelope: None,
            })
            .execute(session)
            {
                Ok(()) => {
                    app.set_status("Message sent");
                    app.cancel_compose();
                }
                Err(e) => app.set_status(format!("Send error: {e}")),
            }
        }
        #[cfg(feature = "imap")]
        Backend::Imap(_) => {
            #[cfg(feature = "smtp")]
            {
                let Some(smtp_config) = app.smtp_config.clone() else {
                    app.set_status("SMTP not configured");
                    return;
                };
                match (SmtpMessageSendHandler { raw: mime_bytes }).execute(&smtp_config) {
                    Ok(()) => {
                        app.set_status("Message sent");
                        app.cancel_compose();
                    }
                    Err(e) => app.set_status(format!("Send error: {e}")),
                }
            }
            #[cfg(not(feature = "smtp"))]
            app.set_status("No send backend available");
        }
    }
}

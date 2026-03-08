use std::{collections::HashSet, path::PathBuf};

use anyhow::{bail, Result};
use edtui::{EditorMode, EditorState, Lines};
use mml::template::{self, TemplateCursor};
use pimalaya_toolbox::config::TomlConfig;

use crate::config::{Config, ImapConfig, SmtpConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Mailboxes,
    Envelopes,
    Message,
    Compose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomPanelMode {
    None,
    Message,
    Compose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeAction {
    Read,
    Reply,
    ReplyAll,
    Forward,
    Copy,
    Move,
    Delete,
}

impl EnvelopeAction {
    pub const ALL: [EnvelopeAction; 7] = [
        EnvelopeAction::Read,
        EnvelopeAction::Reply,
        EnvelopeAction::ReplyAll,
        EnvelopeAction::Forward,
        EnvelopeAction::Copy,
        EnvelopeAction::Move,
        EnvelopeAction::Delete,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            EnvelopeAction::Read => "Read",
            EnvelopeAction::Reply => "Reply",
            EnvelopeAction::ReplyAll => "Reply All",
            EnvelopeAction::Forward => "Forward",
            EnvelopeAction::Copy => "Copy",
            EnvelopeAction::Move => "Move",
            EnvelopeAction::Delete => "Mark for deletion",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeAction {
    Send,
    Preview,
    SaveToDrafts,
    Cancel,
}

impl ComposeAction {
    pub const ALL: [ComposeAction; 4] = [
        ComposeAction::Send,
        ComposeAction::Preview,
        ComposeAction::SaveToDrafts,
        ComposeAction::Cancel,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ComposeAction::Send => "Send",
            ComposeAction::Preview => "Preview",
            ComposeAction::SaveToDrafts => "Save to Drafts",
            ComposeAction::Cancel => "Cancel",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Envelope,
    Compose,
    CopyTo,
    MoveTo,
    Delete,
}

#[derive(Debug, Clone)]
pub struct Mailbox {
    pub name: String,
    pub delimiter: Option<char>,
    pub subscribed: bool,
}

#[derive(Debug, Clone)]
pub struct Envelope {
    pub uid: u32,
    pub date: String,
    pub from: String,
    pub subject: String,
    pub flags: HashSet<String>,
}

pub struct App {
    pub running: bool,
    pub active_panel: Panel,
    pub mailboxes: Vec<Mailbox>,
    pub mailbox_index: usize,
    pub envelopes: Vec<Envelope>,
    pub envelope_index: usize,
    pub selected_mailbox: Option<String>,
    pub account_name: String,
    pub email: String,
    pub display_name: String,
    pub signature: String,
    pub imap_config: ImapConfig,
    pub smtp_config: Option<SmtpConfig>,
    pub status_message: Option<String>,

    // Message viewing
    pub bottom_panel_mode: BottomPanelMode,
    pub message_content: Option<String>,
    pub message_scroll: u16,
    pub previewing_compose: bool,

    // Message composition
    pub editor_state: EditorState,

    // Dialog
    pub dialog: Option<Dialog>,
    pub dialog_index: usize,
}

impl App {
    pub fn new(config_paths: &[PathBuf], account_name: Option<&str>) -> Result<Self> {
        let config = Config::from_paths_or_default(config_paths)?;
        let (name, account_config) = config.get_account(account_name)?;
        let Some(imap_config) = account_config.imap else {
            bail!("IMAP config is missing for this account")
        };

        let email = account_config.email.clone();
        let display_name = account_config
            .display_name
            .or(config.display_name)
            .unwrap_or_default();
        let signature = account_config
            .signature
            .or(config.signature)
            .unwrap_or_default();

        let smtp_config = account_config.smtp;

        Ok(Self {
            running: true,
            active_panel: Panel::Mailboxes,
            mailboxes: Vec::new(),
            mailbox_index: 0,
            envelopes: Vec::new(),
            envelope_index: 0,
            selected_mailbox: None,
            account_name: name,
            email,
            display_name,
            signature,
            imap_config,
            smtp_config,
            status_message: Some("Loading mailboxes...".to_string()),
            bottom_panel_mode: BottomPanelMode::None,
            message_content: None,
            message_scroll: 0,
            previewing_compose: false,
            editor_state: EditorState::new(Lines::from("")),
            dialog: None,
            dialog_index: 0,
        })
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    /// Close the current "frame" - returns true if something was closed
    pub fn close_current(&mut self) -> bool {
        match self.active_panel {
            Panel::Message | Panel::Compose => {
                self.close_bottom_panel();
                true
            }
            Panel::Envelopes => {
                if self.bottom_panel_mode != BottomPanelMode::None {
                    self.close_bottom_panel();
                } else {
                    self.unselect_mailbox();
                }
                true
            }
            _ => false,
        }
    }

    pub fn unselect_mailbox(&mut self) {
        self.selected_mailbox = None;
        self.envelopes.clear();
        self.envelope_index = 0;
        self.close_bottom_panel();
        self.active_panel = Panel::Mailboxes;
    }

    pub fn toggle_panel(&mut self) {
        self.active_panel = match self.active_panel {
            Panel::Mailboxes => Panel::Envelopes,
            Panel::Envelopes => {
                if self.bottom_panel_mode == BottomPanelMode::Message {
                    Panel::Message
                } else if self.bottom_panel_mode == BottomPanelMode::Compose {
                    Panel::Compose
                } else {
                    Panel::Mailboxes
                }
            }
            Panel::Message => Panel::Mailboxes,
            Panel::Compose => Panel::Mailboxes,
        };
    }

    pub fn next_item(&mut self) {
        match self.active_panel {
            Panel::Mailboxes => {
                if !self.mailboxes.is_empty() {
                    self.mailbox_index = (self.mailbox_index + 1) % self.mailboxes.len();
                }
            }
            Panel::Envelopes => {
                if !self.envelopes.is_empty() {
                    self.envelope_index = (self.envelope_index + 1) % self.envelopes.len();
                }
            }
            Panel::Message => {
                self.message_scroll = self.message_scroll.saturating_add(1);
            }
            Panel::Compose => {}
        }
    }

    pub fn previous_item(&mut self) {
        match self.active_panel {
            Panel::Mailboxes => {
                if !self.mailboxes.is_empty() {
                    self.mailbox_index = self
                        .mailbox_index
                        .checked_sub(1)
                        .unwrap_or(self.mailboxes.len() - 1);
                }
            }
            Panel::Envelopes => {
                if !self.envelopes.is_empty() {
                    self.envelope_index = self
                        .envelope_index
                        .checked_sub(1)
                        .unwrap_or(self.envelopes.len() - 1);
                }
            }
            Panel::Message => {
                self.message_scroll = self.message_scroll.saturating_sub(1);
            }
            Panel::Compose => {}
        }
    }

    pub fn select_mailbox(&mut self) {
        let mailbox_name = self
            .mailboxes
            .get(self.mailbox_index)
            .map(|m| m.name.clone());

        if let Some(name) = mailbox_name {
            self.selected_mailbox = Some(name.clone());
            self.envelope_index = 0;
            self.envelopes.clear();
            self.close_bottom_panel();
            self.active_panel = Panel::Envelopes;
            self.status_message = Some(format!("Loading envelopes from {}...", name));
        }
    }

    pub fn set_mailboxes(&mut self, mailboxes: Vec<Mailbox>) {
        self.mailboxes = mailboxes;
        self.mailbox_index = 0;
        if !self.mailboxes.is_empty() {
            self.select_mailbox();
        }
        self.status_message = None;
    }

    pub fn set_envelopes(&mut self, envelopes: Vec<Envelope>) {
        self.envelopes = envelopes;
        self.envelope_index = 0;
        self.status_message = None;
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
    }

    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    pub fn show_message(&mut self, content: String) {
        self.message_content = Some(content);
        self.message_scroll = 0;
        self.bottom_panel_mode = BottomPanelMode::Message;
        self.active_panel = Panel::Message;
    }

    pub fn close_bottom_panel(&mut self) {
        self.bottom_panel_mode = BottomPanelMode::None;
        self.message_content = None;
        self.previewing_compose = false;
        self.dialog = None;
        if self.active_panel == Panel::Message || self.active_panel == Panel::Compose {
            self.active_panel = Panel::Envelopes;
        }
    }

    pub fn preview_compose(&mut self, content: String) {
        self.message_content = Some(content);
        self.message_scroll = 0;
        self.bottom_panel_mode = BottomPanelMode::Message;
        self.active_panel = Panel::Message;
        self.previewing_compose = true;
    }

    pub fn close_preview(&mut self) {
        self.message_content = None;
        self.message_scroll = 0;
        self.previewing_compose = false;
        self.bottom_panel_mode = BottomPanelMode::Compose;
        self.active_panel = Panel::Compose;
    }

    pub fn start_compose(&mut self) {
        let tpl = template::new::build(template::new::BuildNewTemplateArgs {
            from: self.email.clone(),
            from_name: self.display_name.clone(),
            signature: self.signature.clone(),
            ..Default::default()
        });

        match tpl {
            Ok(tpl) => self.open_editor_with_template(&tpl.content, &tpl.cursor),
            Err(err) => self.set_status(format!("Error building template: {err}")),
        }
    }

    pub fn start_reply(&mut self, raw_message: &[u8], reply_all: bool) {
        let Some(msg) = mail_parser::MessageParser::default().parse(raw_message) else {
            self.set_status("Error: failed to parse message");
            return;
        };

        let tpl = template::reply::build(
            &msg,
            template::reply::BuildReplyTemplateArgs {
                from: self.email.clone(),
                from_name: self.display_name.clone(),
                signature: self.signature.clone(),
                reply_all,
                ..Default::default()
            },
        );

        match tpl {
            Ok(tpl) => self.open_editor_with_template(&tpl.content, &tpl.cursor),
            Err(err) => self.set_status(format!("Error building reply template: {err}")),
        }
    }

    pub fn start_forward(&mut self, raw_message: &[u8]) {
        let Some(msg) = mail_parser::MessageParser::default().parse(raw_message) else {
            self.set_status("Error: failed to parse message");
            return;
        };

        let tpl = template::forward::build(
            &msg,
            template::forward::BuildForwardTemplateArgs {
                from: self.email.clone(),
                from_name: self.display_name.clone(),
                signature: self.signature.clone(),
                ..Default::default()
            },
        );

        match tpl {
            Ok(tpl) => self.open_editor_with_template(&tpl.content, &tpl.cursor),
            Err(err) => self.set_status(format!("Error building forward template: {err}")),
        }
    }

    fn open_editor_with_template(&mut self, content: &str, cursor: &TemplateCursor) {
        let mut state = EditorState::new(Lines::from(content));
        state.mode = EditorMode::Insert;
        state.cursor = edtui::Index2::new(cursor.row.saturating_sub(1), cursor.col);
        self.editor_state = state;
        self.bottom_panel_mode = BottomPanelMode::Compose;
        self.active_panel = Panel::Compose;
        self.dialog = None;
    }

    pub fn get_compose_content(&self) -> String {
        self.editor_state.lines.to_string()
    }

    pub fn cancel_compose(&mut self) {
        self.dialog = None;
        self.close_bottom_panel();
    }

    pub fn get_selected_envelope(&self) -> Option<&Envelope> {
        self.envelopes.get(self.envelope_index)
    }

    pub fn remove_selected_envelope(&mut self) {
        if self.envelope_index < self.envelopes.len() {
            self.envelopes.remove(self.envelope_index);
            if self.envelope_index >= self.envelopes.len() && self.envelope_index > 0 {
                self.envelope_index -= 1;
            }
        }
    }

    pub fn flag_selected_envelope(&mut self, flag: &str) {
        if let Some(envelope) = self.envelopes.get_mut(self.envelope_index) {
            envelope.flags.insert(flag.to_string());
        }
    }

    // Dialog

    pub fn open_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
        self.dialog_index = 0;
    }

    pub fn close_dialog(&mut self) {
        self.dialog = None;
    }

    pub fn dialog_item_count(&self) -> usize {
        match self.dialog {
            Some(Dialog::Envelope) => EnvelopeAction::ALL.len(),
            Some(Dialog::Compose) => ComposeAction::ALL.len(),
            Some(Dialog::CopyTo) | Some(Dialog::MoveTo) => self.mailboxes.len(),
            Some(Dialog::Delete) => 2,
            None => 0,
        }
    }

    pub fn dialog_next(&mut self) {
        let count = self.dialog_item_count();
        if count > 0 {
            self.dialog_index = (self.dialog_index + 1) % count;
        }
    }

    pub fn dialog_previous(&mut self) {
        let count = self.dialog_item_count();
        if count > 0 {
            self.dialog_index = self.dialog_index.checked_sub(1).unwrap_or(count - 1);
        }
    }

    pub fn get_selected_envelope_action(&self) -> EnvelopeAction {
        EnvelopeAction::ALL[self.dialog_index]
    }

    pub fn get_selected_compose_action(&self) -> ComposeAction {
        ComposeAction::ALL[self.dialog_index]
    }
}

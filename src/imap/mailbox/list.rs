use anyhow::{bail, Result};
use io_imap::coroutines::lsub::{ImapLsub, ImapLsubResult};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::imap::ImapSession;

use crate::app::Mailbox;

pub struct ImapMailboxListHandler;

impl ImapMailboxListHandler {
    pub fn execute(self, session: &mut ImapSession) -> Result<Vec<Mailbox>> {
        let reference = "".try_into()?;
        let pattern = "*".try_into()?;

        let context = std::mem::take(&mut session.context);
        let mut arg = None;
        let mut coroutine = ImapLsub::new(context, reference, pattern);

        let mailboxes = loop {
            match coroutine.resume(arg.take()) {
                ImapLsubResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                ImapLsubResult::Ok { context, mailboxes } => {
                    session.context = context;
                    break mailboxes;
                }
                ImapLsubResult::Err { context, err } => {
                    session.context = context;
                    bail!(err);
                }
            }
        };

        let result = mailboxes
            .into_iter()
            .map(|(mbox, delim, _attrs)| {
                let name = match mbox {
                    io_imap::types::mailbox::Mailbox::Inbox => "INBOX".to_string(),
                    io_imap::types::mailbox::Mailbox::Other(mbox) => {
                        String::from_utf8_lossy(mbox.inner().as_ref()).to_string()
                    }
                };
                let delimiter = delim.map(|d| d.inner());

                Mailbox {
                    id: None,
                    name,
                    delimiter,
                    subscribed: true,
                }
            })
            .collect();

        Ok(result)
    }
}

use std::num::NonZeroU32;

use anyhow::{anyhow, bail, Result};
use io_imap::{
    coroutines::{
        r#move::{ImapMove, ImapMoveResult},
        select::{ImapSelect, ImapSelectResult},
    },
    types::sequence::SequenceSet,
};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::imap::ImapSession;

pub struct ImapMessageMoveHandler {
    pub mailbox: String,
    pub id: String,
    pub target: String,
}

impl ImapMessageMoveHandler {
    pub fn execute(self, session: &mut ImapSession) -> Result<()> {
        let mailbox_name = self.mailbox.try_into()?;
        let uid: u32 = self.id.parse()?;
        let id = NonZeroU32::new(uid).ok_or_else(|| anyhow!("UID must be non-zero"))?;

        let context = std::mem::take(&mut session.context);
        let mut arg = None;
        let mut coroutine = ImapSelect::new(context, mailbox_name);

        let context = loop {
            match coroutine.resume(arg.take()) {
                ImapSelectResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                ImapSelectResult::Ok { context, .. } => break context,
                ImapSelectResult::Err { context, err } => {
                    session.context = context;
                    bail!(err);
                }
            }
        };

        let sequence_set: SequenceSet = id.try_into()?;
        let target_mailbox: io_imap::types::mailbox::Mailbox<'static> = self.target.try_into()?;

        let mut arg = None;
        let mut coroutine = ImapMove::new(context, sequence_set, target_mailbox, true);

        loop {
            match coroutine.resume(arg.take()) {
                ImapMoveResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                ImapMoveResult::Ok { context, .. } => {
                    session.context = context;
                    break;
                }
                ImapMoveResult::Err { context, err } => {
                    session.context = context;
                    bail!(err);
                }
            }
        }

        Ok(())
    }
}

use std::num::NonZeroU32;

use anyhow::{anyhow, bail, Result};
use io_imap::{
    coroutines::{
        select::{ImapSelect, ImapSelectResult},
        store::{ImapStoreSilent, ImapStoreSilentResult},
    },
    types::{
        flag::{Flag, StoreType},
        sequence::SequenceSet,
    },
};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::imap::ImapSession;

pub struct ImapFlagRemoveHandler {
    pub mailbox: String,
    pub id: String,
    pub flags: Vec<Flag<'static>>,
}

impl ImapFlagRemoveHandler {
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

        let mut arg = None;
        let mut coroutine =
            ImapStoreSilent::new(context, sequence_set, StoreType::Remove, self.flags, true);

        loop {
            match coroutine.resume(arg.take()) {
                ImapStoreSilentResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                ImapStoreSilentResult::Ok { context } => {
                    session.context = context;
                    break;
                }
                ImapStoreSilentResult::Err { context, err } => {
                    session.context = context;
                    bail!(err);
                }
            }
        }

        Ok(())
    }
}

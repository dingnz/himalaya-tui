use std::num::NonZeroU32;

use anyhow::{anyhow, bail, Result};
use io_imap::{
    coroutines::{
        fetch::{ImapFetchFirst, ImapFetchFirstResult},
        select::{ImapSelect, ImapSelectResult},
    },
    types::fetch::{MacroOrMessageDataItemNames, MessageDataItem},
};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::imap::ImapSession;

use crate::imap::fetch_item_names_body_peek;

pub struct ImapMessageGetRawHandler {
    pub mailbox: String,
    pub id: String,
}

impl ImapMessageGetRawHandler {
    pub fn execute(self, session: &mut ImapSession) -> Result<Vec<u8>> {
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

        let item_names =
            MacroOrMessageDataItemNames::MessageDataItemNames(fetch_item_names_body_peek());

        let mut arg = None;
        let mut coroutine = ImapFetchFirst::new(context, id, item_names, true);

        let items = loop {
            match coroutine.resume(arg.take()) {
                ImapFetchFirstResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                ImapFetchFirstResult::Ok { context, items } => {
                    session.context = context;
                    break items;
                }
                ImapFetchFirstResult::Err { context, err } => {
                    session.context = context;
                    bail!(err);
                }
            }
        };

        for item in items {
            if let MessageDataItem::BodyExt { data, .. } = item {
                if let Some(data) = data.0 {
                    return Ok(data.as_ref().to_vec());
                }
            }
        }

        Err(anyhow!("No message data returned"))
    }
}

use anyhow::{bail, Result};
use io_imap::{
    coroutines::{
        fetch::{ImapFetch, ImapFetchResult},
        select::{ImapSelect, ImapSelectResult},
    },
    types::{
        fetch::{MacroOrMessageDataItemNames, MessageDataItemName},
        sequence::SequenceSet,
    },
};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::imap::ImapSession;

use crate::app::Envelope;
use crate::imap::parse_envelope;

pub struct ImapEnvelopeListHandler {
    pub mailbox: String,
    pub page: usize,
    pub page_size: usize,
}

impl ImapEnvelopeListHandler {
    pub fn execute(self, session: &mut ImapSession) -> Result<(Vec<Envelope>, u32)> {
        let mailbox_name = self.mailbox.try_into()?;

        let context = std::mem::take(&mut session.context);
        let mut arg = None;
        let mut coroutine = ImapSelect::new(context, mailbox_name);

        let (context, select_data) = loop {
            match coroutine.resume(arg.take()) {
                ImapSelectResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                ImapSelectResult::Ok { context, data } => break (context, data),
                ImapSelectResult::Err { context, err } => {
                    session.context = context;
                    bail!(err);
                }
            }
        };

        let total = select_data.exists.unwrap_or(0);

        if total == 0 {
            session.context = context;
            return Ok((Vec::new(), 0));
        }

        let page_size = self.page_size as u32;
        let offset = (self.page as u32) * page_size;

        if offset >= total {
            session.context = context;
            return Ok((Vec::new(), total));
        }

        // Sequence numbers are 1-based; newest messages have the highest seq numbers.
        let end = total - offset;
        let start = end.saturating_sub(page_size - 1).max(1);

        let sequence_set: SequenceSet = format!("{start}:{end}").parse()?;
        let item_names = MacroOrMessageDataItemNames::MessageDataItemNames(vec![
            MessageDataItemName::Uid,
            MessageDataItemName::Envelope,
            MessageDataItemName::Flags,
        ]);

        let mut arg = None;
        let mut coroutine = ImapFetch::new(context, sequence_set, item_names, false);

        let data = loop {
            match coroutine.resume(arg.take()) {
                ImapFetchResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                ImapFetchResult::Ok { context, data } => {
                    session.context = context;
                    break data;
                }
                ImapFetchResult::Err { context, err } => {
                    session.context = context;
                    bail!(err);
                }
            }
        };

        let mut envelopes: Vec<Envelope> = data
            .into_iter()
            .map(|(seq, items)| parse_envelope(seq.get(), items))
            .collect();

        envelopes.sort_by(|a, b| b.id.cmp(&a.id));

        Ok((envelopes, total))
    }
}

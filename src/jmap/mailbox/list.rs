use anyhow::{bail, Result};
use io_jmap::rfc8621::coroutines::mailbox_query::{JmapMailboxQuery, JmapMailboxQueryResult};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::jmap::JmapSession;

use crate::app::Mailbox;

pub struct JmapMailboxListHandler;

impl JmapMailboxListHandler {
    pub fn execute(self, session: &mut JmapSession) -> Result<Vec<Mailbox>> {
        let mut coroutine = JmapMailboxQuery::new(
            &session.session,
            &session.http_auth,
            None,
            None,
            None,
            None,
            None,
        )?;
        let mut arg = None;

        let mailboxes = loop {
            match coroutine.resume(arg.take()) {
                JmapMailboxQueryResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                JmapMailboxQueryResult::Ok { mailboxes, .. } => break mailboxes,
                JmapMailboxQueryResult::Err { err } => bail!(err),
            }
        };

        let result = mailboxes
            .into_iter()
            .map(|m| Mailbox {
                id: m.id.clone(),
                name: m.name.clone().unwrap_or_default(),
                delimiter: None,
                subscribed: m.is_subscribed,
            })
            .collect();

        Ok(result)
    }
}

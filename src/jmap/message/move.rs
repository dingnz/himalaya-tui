use std::collections::HashMap;

use anyhow::{bail, Result};
use io_jmap::rfc8621::coroutines::email_set::{JmapEmailSet, JmapEmailSetArgs, JmapEmailSetResult};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::jmap::JmapSession;

pub struct JmapMessageMoveHandler {
    pub id: String,
    pub target_mailbox_id: String,
}

impl JmapMessageMoveHandler {
    pub fn execute(self, session: &mut JmapSession) -> Result<()> {
        let mut args = JmapEmailSetArgs::default();
        let new_mailbox_ids = HashMap::from([(self.target_mailbox_id, true)]);
        args.replace_mailbox_ids(self.id, new_mailbox_ids);

        let mut coroutine = JmapEmailSet::new(&session.session, &session.http_auth, args)?;
        let mut arg = None;

        loop {
            match coroutine.resume(arg.take()) {
                JmapEmailSetResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                JmapEmailSetResult::Ok { .. } => break,
                JmapEmailSetResult::Err { err } => bail!(err),
            }
        }

        Ok(())
    }
}

use anyhow::{bail, Result};
use io_jmap::rfc8621::coroutines::email_set::{JmapEmailSet, JmapEmailSetArgs, JmapEmailSetResult};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::jmap::JmapSession;

pub struct JmapMessageDeleteHandler {
    pub id: String,
}

impl JmapMessageDeleteHandler {
    pub fn execute(self, session: &mut JmapSession) -> Result<()> {
        let mut args = JmapEmailSetArgs::default();
        args.destroy(self.id);

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

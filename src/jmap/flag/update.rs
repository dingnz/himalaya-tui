use anyhow::{bail, Result};
use io_jmap::rfc8621::coroutines::email_set::{JmapEmailSet, JmapEmailSetArgs, JmapEmailSetResult};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::jmap::JmapSession;

pub struct JmapFlagUpdateHandler {
    pub id: String,
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

impl JmapFlagUpdateHandler {
    pub fn execute(self, session: &mut JmapSession) -> Result<()> {
        let mut args = JmapEmailSetArgs::default();

        for keyword in self.add {
            args.set_keyword(self.id.clone(), keyword);
        }
        for keyword in self.remove {
            args.unset_keyword(self.id.clone(), keyword);
        }

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

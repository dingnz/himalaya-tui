use anyhow::{bail, Result};
use io_imap::{
    coroutines::append::{ImapAppend, ImapAppendResult},
    types::{core::Literal, extensions::binary::LiteralOrLiteral8, flag::Flag},
};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::imap::ImapSession;

pub struct ImapMessageSaveHandler {
    pub mailbox: String,
    pub raw: Vec<u8>,
    pub flags: Vec<Flag<'static>>,
}

impl ImapMessageSaveHandler {
    pub fn execute(self, session: &mut ImapSession) -> Result<()> {
        let mailbox: io_imap::types::mailbox::Mailbox<'static> = self.mailbox.try_into()?;
        let literal = Literal::try_from(self.raw)?;
        let message = LiteralOrLiteral8::Literal(literal);

        let context = std::mem::take(&mut session.context);
        let mut arg = None;
        let mut coroutine = ImapAppend::new(context, mailbox, self.flags, None, message);

        loop {
            match coroutine.resume(arg.take()) {
                ImapAppendResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                ImapAppendResult::Ok { context, .. } => {
                    session.context = context;
                    break;
                }
                ImapAppendResult::Err { context, err } => {
                    session.context = context;
                    bail!(err);
                }
            }
        }

        Ok(())
    }
}

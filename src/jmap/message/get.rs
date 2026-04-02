use anyhow::{anyhow, bail, Result};
use io_jmap::rfc8621::coroutines::email_get::{JmapEmailGet, JmapEmailGetResult};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::jmap::JmapSession;

pub struct JmapMessageGetHandler {
    pub id: String,
}

impl JmapMessageGetHandler {
    pub fn execute(self, session: &mut JmapSession) -> Result<String> {
        let mut coroutine = JmapEmailGet::new(
            &session.session,
            &session.http_auth,
            vec![self.id],
            None,
            true,
            true,
            0,
        )?;
        let mut arg = None;

        let emails = loop {
            match coroutine.resume(arg.take()) {
                JmapEmailGetResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                JmapEmailGetResult::Ok { emails, .. } => break emails,
                JmapEmailGetResult::Err { err } => bail!(err),
            }
        };

        let email = emails
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Email not found"))?;

        // Try plain text body first
        if let (Some(text_body), Some(body_values)) = (&email.text_body, &email.body_values) {
            if let Some(part) = text_body.first() {
                if let Some(part_id) = &part.part_id {
                    if let Some(bv) = body_values.get(part_id) {
                        return Ok(bv.value.clone());
                    }
                }
            }
        }

        // Fall back to HTML body
        if let (Some(html_body), Some(body_values)) = (&email.html_body, &email.body_values) {
            if let Some(part) = html_body.first() {
                if let Some(part_id) = &part.part_id {
                    if let Some(bv) = body_values.get(part_id) {
                        return Ok(html2text::from_read(bv.value.as_bytes(), 80));
                    }
                }
            }
        }

        Ok(email.preview.clone().unwrap_or_default())
    }
}

use std::collections::HashSet;

use anyhow::{bail, Result};
use io_jmap::rfc8621::{
    coroutines::email_query::{JmapEmailQuery, JmapEmailQueryResult},
    types::email::{EmailComparator, EmailFilter, EmailProperty},
};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::jmap::JmapSession;

use crate::app::Envelope;

pub struct JmapEnvelopeListHandler {
    pub mailbox_id: String,
    pub page: usize,
    pub page_size: usize,
}

impl JmapEnvelopeListHandler {
    pub fn execute(self, session: &mut JmapSession) -> Result<(Vec<Envelope>, u32)> {
        let filter = Some(EmailFilter {
            in_mailbox: Some(self.mailbox_id),
            ..Default::default()
        });
        let sort = Some(vec![EmailComparator::received_at_desc()]);
        let position = Some((self.page * self.page_size) as u64);
        let limit = Some(self.page_size as u64);
        let properties = Some(vec![
            EmailProperty::Id,
            EmailProperty::From,
            EmailProperty::Subject,
            EmailProperty::ReceivedAt,
            EmailProperty::Keywords,
        ]);

        let mut coroutine = JmapEmailQuery::new(
            &session.session,
            &session.http_auth,
            filter,
            sort,
            position,
            limit,
            properties,
        )?;
        let mut arg = None;

        let (emails, total) = loop {
            match coroutine.resume(arg.take()) {
                JmapEmailQueryResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                JmapEmailQueryResult::Ok { emails, total, .. } => break (emails, total),
                JmapEmailQueryResult::Err { err } => bail!(err),
            }
        };

        let envelopes = emails
            .into_iter()
            .map(|email| {
                let id = email.id.clone().unwrap_or_default();

                let from = email
                    .from
                    .as_deref()
                    .and_then(|a| a.first())
                    .map(|a| {
                        a.name
                            .as_deref()
                            .filter(|n| !n.is_empty())
                            .unwrap_or(&a.email)
                            .to_string()
                    })
                    .unwrap_or_default();

                let subject = email.subject.clone().unwrap_or_default();

                let date = email
                    .received_at
                    .as_deref()
                    .map(|s| s.get(..10).unwrap_or(s).to_string())
                    .unwrap_or_default();

                let flags = email
                    .keywords
                    .as_ref()
                    .map(|kw| {
                        kw.iter()
                            .filter_map(|(k, &v)| if v { Some(k.clone()) } else { None })
                            .collect::<HashSet<_>>()
                    })
                    .unwrap_or_default();

                Envelope {
                    id,
                    from,
                    subject,
                    date,
                    flags,
                }
            })
            .collect();

        Ok((envelopes, total.unwrap_or(0) as u32))
    }
}

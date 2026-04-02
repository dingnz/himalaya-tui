use std::collections::HashMap;

use anyhow::{bail, Result};
use io_jmap::{
    rfc8620::{
        coroutines::blob_upload::{JmapBlobUpload, JmapBlobUploadResult},
        types::session::capabilities,
    },
    rfc8621::{
        coroutines::{
            email_import::{JmapEmailImport, JmapEmailImportResult},
            email_submission_set::{JmapEmailSubmissionSet, JmapEmailSubmissionSetResult},
            identity_get::{JmapIdentityGet, JmapIdentityGetResult},
        },
        types::{
            email::EmailImport,
            email_submission::{EmailSubmissionCreate, Envelope},
        },
    },
};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::jmap::JmapSession;
use url::Url;

pub struct JmapMessageSendHandler {
    pub raw: Vec<u8>,
    /// ID of the Sent mailbox to store the outgoing message in.
    /// JMAP requires at least one mailbox for `Email/import`.
    pub sent_mailbox_id: Option<String>,
    /// Explicit SMTP envelope override. `None` means derive from message headers.
    pub envelope: Option<Envelope>,
}

impl JmapMessageSendHandler {
    pub fn execute(self, session: &mut JmapSession) -> Result<()> {
        // 1. Resolve upload URL (RFC 8620 §6.1 — `{accountId}` template variable).
        let account_id = session
            .session
            .primary_accounts
            .get(capabilities::MAIL)
            .map(|s| s.as_str())
            .unwrap_or("");
        let upload_url: Url = session
            .session
            .upload_url
            .replace("{accountId}", account_id)
            .parse()?;

        // 2. Upload raw MIME bytes as a blob.
        let mut coroutine =
            JmapBlobUpload::new(&session.http_auth, &upload_url, "message/rfc822", self.raw)?;
        let mut arg = None;
        let blob_id = loop {
            match coroutine.resume(arg.take()) {
                JmapBlobUploadResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                JmapBlobUploadResult::Ok { blob_id, .. } => break blob_id,
                JmapBlobUploadResult::Err { err } => bail!(err),
            }
        };

        // 3. Fetch the first available identity (needed for submission).
        let mut coroutine = JmapIdentityGet::new(&session.session, &session.http_auth, None)?;
        let mut arg = None;
        let identity_id = loop {
            match coroutine.resume(arg.take()) {
                JmapIdentityGetResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                JmapIdentityGetResult::Ok { identities, .. } => {
                    let id = identities
                        .into_iter()
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("No JMAP identities found"))?
                        .id;
                    break id;
                }
                JmapIdentityGetResult::Err { err } => bail!(err),
            }
        };

        // 4. Import the blob as an Email object.
        //    `EmailSubmission/set` requires an existing email ID, so we import
        //    it into the Sent mailbox (or bail if none is known).
        let mailbox_ids: HashMap<String, bool> = match self.sent_mailbox_id {
            Some(id) => [(id, true)].into_iter().collect(),
            None => {
                bail!("No Sent mailbox ID available; cannot import message before JMAP submission")
            }
        };

        let import = EmailImport {
            blob_id: blob_id.clone(),
            mailbox_ids,
            keywords: Some([("$seen".to_string(), true)].into_iter().collect()),
            received_at: None,
        };

        let mut emails = HashMap::new();
        emails.insert("send".to_string(), import);

        let mut coroutine = JmapEmailImport::new(&session.session, &session.http_auth, emails)?;
        let mut arg = None;
        let (created, not_created) = loop {
            match coroutine.resume(arg.take()) {
                JmapEmailImportResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                JmapEmailImportResult::Ok {
                    created,
                    not_created,
                    ..
                } => break (created, not_created),
                JmapEmailImportResult::Err { err } => bail!(err),
            }
        };

        if let Some(err) = not_created.get("send") {
            let desc = err.description.as_deref().unwrap_or("unknown error");
            bail!("JMAP Email/import failed: {desc}");
        }

        let email_id = created
            .get("send")
            .and_then(|e| e.id.clone())
            .ok_or_else(|| anyhow::anyhow!("Email/import succeeded but no email ID returned"))?;

        // 5. Submit via EmailSubmission/set.
        let submission = EmailSubmissionCreate {
            identity_id,
            email_id: email_id.clone(),
            envelope: self.envelope,
        };

        let mut submissions = HashMap::new();
        submissions.insert("send".to_string(), submission);

        let mut coroutine =
            JmapEmailSubmissionSet::new(&session.session, &session.http_auth, submissions)?;
        let mut arg = None;
        let not_created = loop {
            match coroutine.resume(arg.take()) {
                JmapEmailSubmissionSetResult::Io { io } => {
                    arg = Some(handle(&mut session.stream, io)?)
                }
                JmapEmailSubmissionSetResult::Ok { not_created, .. } => break not_created,
                JmapEmailSubmissionSetResult::Err { err } => bail!(err),
            }
        };

        if let Some(err) = not_created.get("send") {
            let desc = err.description.as_deref().unwrap_or("unknown error");
            bail!("JMAP EmailSubmission/set failed: {desc}");
        }

        Ok(())
    }
}

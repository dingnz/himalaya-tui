use std::collections::HashMap;

use anyhow::{bail, Result};
use io_jmap::rfc8620::coroutines::blob_upload::{JmapBlobUpload, JmapBlobUploadResult};
use io_jmap::rfc8620::types::session::capabilities;
use io_jmap::rfc8621::{
    coroutines::email_import::{JmapEmailImport, JmapEmailImportResult},
    types::email::EmailImport,
};
use io_stream::runtimes::std::handle;
use pimalaya_toolbox::stream::jmap::JmapSession;
use url::Url;

pub struct JmapMessageSaveHandler {
    pub mailbox_id: String,
    pub raw: Vec<u8>,
}

impl JmapMessageSaveHandler {
    pub fn execute(self, session: &mut JmapSession) -> Result<()> {
        // 1. Resolve upload URL.
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

        // 2. Upload raw bytes as a blob.
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

        // 3. Import blob into the Drafts mailbox with the $draft keyword.
        let import = EmailImport {
            blob_id: blob_id.clone(),
            mailbox_ids: [(self.mailbox_id, true)].into_iter().collect(),
            keywords: Some([("$draft".to_string(), true)].into_iter().collect()),
            received_at: None,
        };

        let mut emails = HashMap::new();
        emails.insert("draft".to_string(), import);

        let mut coroutine = JmapEmailImport::new(&session.session, &session.http_auth, emails)?;
        let mut arg = None;
        let not_created = loop {
            match coroutine.resume(arg.take()) {
                JmapEmailImportResult::Io { io } => arg = Some(handle(&mut session.stream, io)?),
                JmapEmailImportResult::Ok { not_created, .. } => break not_created,
                JmapEmailImportResult::Err { err } => bail!(err),
            }
        };

        if let Some(err) = not_created.get("draft") {
            let desc = err.description.as_deref().unwrap_or("unknown error");
            bail!("JMAP Email/import (draft) failed: {desc}");
        }

        Ok(())
    }
}

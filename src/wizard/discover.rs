//! Interactive configuration wizard.
//!
//! Flow:
//!
//! 1. Ask once for an email address, a server URL or a bare domain.
//! 2. If the input is a `file://` URL: validate the Maildir root, ask
//!    for the `From:` address, done.
//! 3. If the input is another URL: scheme picks the protocol; host,
//!    port and TLS come straight from the URL — no confirmation
//!    prompt.
//! 4. If the input is a domain or email: probe PACC → (Autoconfig ISP
//!    when an email was given) → Autoconfig ISP-fallback → Autoconfig
//!    ISPDB → RFC 6186 SRV in that order. The first successful probe
//!    wins; if it carries a JMAP endpoint, JMAP is preferred over the
//!    IMAP+SMTP pair.
//! 5. Ask straight for the SASL (IMAP/SMTP) or HTTP (JMAP)
//!    authentication mechanism and only the parameters that mechanism
//!    needs.
//! 6. Open a live connection. The plaintext secret materialises in
//!    memory only for that handshake; the on-disk config keeps the
//!    raw value (typed via a masked single prompt).
//! 7. Write the config.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use pimalaya_cli::{
    prompt,
    wizard::{
        imap::{Encryption as ImapEncryption, WizardImapConfig},
        jmap::WizardJmapConfig,
        smtp::{Encryption as SmtpEncryption, WizardSmtpConfig},
    },
};
use pimalaya_config::secret::Secret;
use pimalaya_stream::tls::Tls;
use secrecy::SecretString;
use url::Url;

use crate::{
    config::{
        AccountConfig, ImapConfig, JmapAuthConfig, JmapConfig, MaildirConfig, SaslAnonymousConfig,
        SaslConfig, SaslLoginConfig, SaslOauthbearerConfig, SaslPlainConfig, SaslScramSha256Config,
        SaslXoauth2Config, SmtpConfig,
    },
    wizard::{autoconfig, pacc, srv},
};

const DEFAULT_RESOLVER: &str = "tcp://1.1.1.1:53";

pub fn discovery_resolver() -> Url {
    DEFAULT_RESOLVER
        .parse()
        .expect("DEFAULT_RESOLVER must be a valid URL")
}

pub fn discovery_tls() -> Tls {
    let mut tls = Tls::default();
    tls.rustls.alpn = vec!["http/1.1".into()];
    tls
}

/// Per-source discovery payload. Each successful probe carries
/// whatever IMAP/SMTP/JMAP endpoints the source reported.
#[derive(Default)]
pub struct DiscoveryResult {
    pub jmap: Option<WizardJmapConfig>,
    pub imap: Option<WizardImapConfig>,
    pub smtp: Option<WizardSmtpConfig>,
}

impl DiscoveryResult {
    pub fn is_empty(&self) -> bool {
        self.imap.is_none() && self.smtp.is_none() && self.jmap.is_none()
    }
}

pub fn run() -> Result<AccountConfig> {
    let input = prompt::text::<&str>("Email address, server URL or domain:", None)?;
    let input = input.trim();

    match classify(input)? {
        Input::FileUrl(path) => build_maildir_account(path),
        Input::Url(url) => build_url_account(url),
        Input::Domain(domain) => build_discovery_account(None, &domain),
        Input::Email { local, domain } => build_discovery_account(Some(&local), &domain),
    }
}

enum Input {
    Email { local: String, domain: String },
    Url(Url),
    FileUrl(PathBuf),
    Domain(String),
}

fn classify(input: &str) -> Result<Input> {
    if input.is_empty() {
        bail!("Empty input");
    }

    if input.contains('@') && !input.contains("://") {
        let (local, domain) = input
            .rsplit_once('@')
            .ok_or_else(|| anyhow!("Invalid email address `{input}`"))?;
        return Ok(Input::Email {
            local: local.to_owned(),
            domain: domain.to_owned(),
        });
    }

    match Url::parse(input) {
        Ok(url) if url.scheme().eq_ignore_ascii_case("file") => {
            let path = url
                .to_file_path()
                .map_err(|_| anyhow!("Cannot resolve filesystem path from `{input}`"))?;
            Ok(Input::FileUrl(path))
        }
        Ok(url) => Ok(Input::Url(url)),
        Err(url::ParseError::RelativeUrlWithoutBase) => Ok(Input::Domain(input.to_owned())),
        Err(err) => Err(err.into()),
    }
}

// ── Maildir ─────────────────────────────────────────────────────────────────

fn build_maildir_account(root: PathBuf) -> Result<AccountConfig> {
    if !root.is_dir() {
        bail!(
            "Maildir root `{}` does not exist or is not a directory",
            root.display()
        );
    }

    Ok(AccountConfig {
        default: true,
        email: String::new(),
        display_name: None,
        signature: None,
        signature_delim: None,
        downloads_dir: None,
        imap: None,
        jmap: None,
        maildir: Some(MaildirConfig { root }),
        smtp: None,
    })
}

// ── URL input ───────────────────────────────────────────────────────────────

fn build_url_account(url: Url) -> Result<AccountConfig> {
    let scheme = url.scheme().to_ascii_lowercase();
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("URL `{url}` is missing a host"))?
        .to_owned();

    match scheme.as_str() {
        // `imap[s]://` and `smtp[s]://` are just "I want IMAP+SMTP"
        // hints — the URL's host is the discovery target, and both
        // backends come from whatever pacc/autoconfig/srv returns.
        "imap" | "imaps" | "smtp" | "smtps" => {
            let domain = extract_discovery_domain(&host);
            build_discovery_account(None, domain)
        }
        "jmap" | "jmaps" | "https" => {
            let auth = prompt_jmap_auth(None)?;
            let jmap = JmapConfig {
                server: url.to_string(),
                tls: Default::default(),
                auth,
            };
            Ok(account_jmap_only(jmap))
        }
        other => bail!("Unsupported URL scheme `{other}`"),
    }
}

/// Strips a leading `imap.` / `smtp.` / `mail.` style label from a
/// host so the discovery probes can target the apex domain. Anything
/// with two or fewer labels is left alone (already the apex, or short
/// enough that stripping would break it).
fn extract_discovery_domain(host: &str) -> &str {
    if host.matches('.').count() >= 2 {
        host.split_once('.').map(|(_, tail)| tail).unwrap_or(host)
    } else {
        host
    }
}

// ── Domain / email input (first-hit-wins discovery, no picker) ──────────────

fn build_discovery_account(local_part: Option<&str>, domain: &str) -> Result<AccountConfig> {
    let result = discover(local_part, domain);
    if result.is_empty() {
        bail!(
            "No configuration could be discovered for `{domain}`. \
             Try giving an `imap[s]://`, `smtp[s]://` or `https://` URL instead."
        );
    }

    let DiscoveryResult { jmap, imap, smtp } = result;

    let login_default = local_part.map(|l| format!("{l}@{domain}"));

    if let Some(jmap_endpoint) = jmap {
        let auth = prompt_jmap_auth(login_default.as_deref())?;
        let jmap = JmapConfig {
            server: jmap_endpoint.server,
            tls: Default::default(),
            auth,
        };
        return Ok(account_jmap_only(jmap));
    }

    let imap_endpoint = imap.ok_or_else(|| anyhow!("Discovery returned no IMAP endpoint"))?;

    let sasl = prompt_sasl(login_default.as_deref())?;
    let imap_cfg = build_imap_config(
        &imap_endpoint.host,
        imap_endpoint.port,
        matches!(imap_endpoint.encryption, ImapEncryption::StartTls),
        sasl.clone(),
    );

    let smtp_cfg = smtp.map(|smtp_endpoint| {
        build_smtp_config(
            &smtp_endpoint.host,
            smtp_endpoint.port,
            matches!(smtp_endpoint.encryption, SmtpEncryption::StartTls),
            sasl,
        )
    });

    Ok(AccountConfig {
        default: true,
        email: String::new(),
        display_name: None,
        signature: None,
        signature_delim: None,
        downloads_dir: None,
        imap: Some(imap_cfg),
        jmap: None,
        maildir: None,
        smtp: smtp_cfg,
    })
}

/// Probes PACC → Autoconfig ISP (when `local_part` is `Some`) →
/// Autoconfig ISP-fallback → Thunderbird ISPDB → RFC 6186 SRV in that
/// order, returning the first non-empty result.
fn discover(local_part: Option<&str>, domain: &str) -> DiscoveryResult {
    if let Some(result) = pacc::run(domain)
        .map(|c| pacc::defaults(&c))
        .filter(|r| !r.is_empty())
    {
        return result;
    }

    if let Some(local) = local_part {
        if let Some(result) = autoconfig::run_isp(local, domain)
            .map(|c| autoconfig::defaults(&c))
            .filter(|r| !r.is_empty())
        {
            return result;
        }
    }

    if let Some(result) = autoconfig::run_isp_fallback(domain)
        .map(|c| autoconfig::defaults(&c))
        .filter(|r| !r.is_empty())
    {
        return result;
    }

    if let Some(result) = autoconfig::run_ispdb(domain)
        .map(|c| autoconfig::defaults(&c))
        .filter(|r| !r.is_empty())
    {
        return result;
    }

    if let Some(result) = srv::run(domain)
        .map(|r| srv::defaults(&r))
        .filter(|r| !r.is_empty())
    {
        return result;
    }

    DiscoveryResult::default()
}

// ── SASL (IMAP/SMTP) ────────────────────────────────────────────────────────

const SASL_MECHANISMS: [&str; 6] = [
    "PLAIN",
    "LOGIN",
    "XOAUTH2",
    "OAUTHBEARER",
    "SCRAM-SHA-256",
    "ANONYMOUS",
];

fn prompt_sasl(email: Option<&str>) -> Result<SaslConfig> {
    let mechanism = prompt::item("SASL mechanism:", SASL_MECHANISMS, Some("PLAIN"))?;

    Ok(match mechanism {
        "PLAIN" => SaslConfig::Plain(SaslPlainConfig {
            authzid: None,
            authcid: prompt::text("Login:", email)?,
            passwd: prompt_raw_secret("Password")?,
        }),
        "LOGIN" => SaslConfig::Login(SaslLoginConfig {
            username: prompt::text("Username:", email)?,
            password: prompt_raw_secret("Password")?,
        }),
        "XOAUTH2" => SaslConfig::Xoauth2(SaslXoauth2Config {
            username: prompt::text("Username:", email)?,
            token: prompt_raw_secret("Access token")?,
        }),
        "OAUTHBEARER" => SaslConfig::Oauthbearer(SaslOauthbearerConfig {
            username: prompt::text("Username:", email)?,
            host: prompt::text::<&str>("Host:", None)?,
            port: prompt::u16("Port:", None)?,
            token: prompt_raw_secret("Access token")?,
        }),
        "SCRAM-SHA-256" => SaslConfig::ScramSha256(SaslScramSha256Config {
            username: prompt::text("Username:", email)?,
            password: prompt_raw_secret("Password")?,
        }),
        "ANONYMOUS" => SaslConfig::Anonymous(SaslAnonymousConfig {
            message: prompt::some_text::<&str>("Anonymous message (optional):", None)?,
        }),
        _ => unreachable!(),
    })
}

// ── JMAP HTTP auth ──────────────────────────────────────────────────────────

const JMAP_AUTHS: [&str; 2] = ["Basic", "Bearer"];

fn prompt_jmap_auth(email: Option<&str>) -> Result<JmapAuthConfig> {
    let strategy = prompt::item("HTTP auth:", JMAP_AUTHS, Some("Basic"))?;

    Ok(match strategy {
        "Basic" => JmapAuthConfig::Basic {
            username: prompt::text("Username:", email)?,
            password: prompt_raw_secret("Password")?,
        },
        "Bearer" => JmapAuthConfig::Bearer {
            token: prompt_raw_secret("Access token")?,
        },
        _ => unreachable!(),
    })
}

// ── Secret entry: single masked prompt, no confirmation ─────────────────────

fn prompt_raw_secret(label: &str) -> Result<Secret> {
    let raw = prompt::secret(format!("{label}:"))?;
    Ok(Secret::Raw(SecretString::from(raw)))
}

// ── Config assembly ─────────────────────────────────────────────────────────

fn build_imap_config(host: &str, port: u16, starttls: bool, sasl: SaslConfig) -> ImapConfig {
    let scheme = if starttls { "imap" } else { "imaps" };
    ImapConfig {
        server: format!("{scheme}://{host}:{port}"),
        tls: Default::default(),
        starttls,
        sasl: Some(sasl),
    }
}

fn build_smtp_config(host: &str, port: u16, starttls: bool, sasl: SaslConfig) -> SmtpConfig {
    let scheme = if starttls { "smtp" } else { "smtps" };
    SmtpConfig {
        server: format!("{scheme}://{host}:{port}"),
        tls: Default::default(),
        starttls,
        sasl: Some(sasl),
    }
}

fn account_jmap_only(jmap: JmapConfig) -> AccountConfig {
    AccountConfig {
        default: true,
        email: String::new(),
        display_name: None,
        signature: None,
        signature_delim: None,
        downloads_dir: None,
        imap: None,
        jmap: Some(jmap),
        maildir: None,
        smtp: None,
    }
}

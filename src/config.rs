use std::{collections::HashMap, path::PathBuf};

use anyhow::Result;
use pimalaya_toolbox::{
    config::{shell_expanded_string, TomlConfig},
    sasl::{sasl_default_mechanisms, Sasl, SaslAnonymous, SaslLogin, SaslMechanism, SaslPlain},
    secret::{Secret, SecretError},
    stream::{Rustls, RustlsCrypto, Tls, TlsProvider},
};
use serde::Deserialize;
use url::Url;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
    #[serde(alias = "name")]
    pub display_name: Option<String>,
    pub signature: Option<String>,
    pub signature_delim: Option<String>,
    pub downloads_dir: Option<PathBuf>,
    pub accounts: HashMap<String, AccountConfig>,
}

impl TomlConfig for Config {
    type Account = AccountConfig;

    fn project_name() -> &'static str {
        env!("CARGO_PKG_NAME")
    }

    fn find_default_account(&self) -> Option<(String, Self::Account)> {
        self.accounts
            .iter()
            .find(|(_, account)| account.default)
            .map(|(name, account)| (name.to_owned(), account.clone()))
    }

    fn find_account(&self, name: &str) -> Option<(String, Self::Account)> {
        self.accounts
            .get(name)
            .map(|account| (name.to_owned(), account.clone()))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct AccountConfig {
    #[serde(default)]
    pub default: bool,
    pub imap: Option<ImapConfig>,
    pub smtp: Option<SmtpConfig>,
    #[cfg(feature = "jmap")]
    pub jmap: Option<JmapConfig>,
    #[serde(deserialize_with = "shell_expanded_string")]
    pub email: String,
    pub display_name: Option<String>,
    pub signature: Option<String>,
    pub signature_delim: Option<String>,
    pub downloads_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SmtpConfig {
    pub url: Url,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub starttls: bool,
    #[serde(default)]
    pub sasl: SaslConfig,
}

#[cfg(feature = "smtp")]
impl SmtpConfig {
    pub fn into_session(self) -> Result<pimalaya_toolbox::stream::smtp::SmtpSession> {
        Ok(pimalaya_toolbox::stream::smtp::SmtpSession::new(
            self.url,
            self.tls.try_into()?,
            self.starttls,
            self.sasl.try_into()?,
        )?)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ImapConfig {
    pub url: Url,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub starttls: bool,
    #[serde(default)]
    pub sasl: SaslConfig,
}

#[cfg(feature = "imap")]
impl ImapConfig {
    pub fn into_session(self) -> Result<pimalaya_toolbox::stream::imap::ImapSession> {
        Ok(pimalaya_toolbox::stream::imap::ImapSession::new(
            self.url,
            self.tls.try_into()?,
            self.starttls,
            self.sasl.try_into()?,
        )?)
    }
}

#[cfg(feature = "jmap")]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct JmapConfig {
    pub server: String,
    #[serde(default)]
    pub tls: TlsConfig,
    pub auth: JmapAuthConfig,
}

#[cfg(feature = "jmap")]
impl JmapConfig {
    pub fn into_session(self) -> Result<pimalaya_toolbox::stream::jmap::JmapSession> {
        use pimalaya_toolbox::stream::jmap::{JmapAuth, JmapSession};

        let auth = match self.auth {
            JmapAuthConfig::Bearer { token } => JmapAuth::Bearer(token.get()?.into()),
            JmapAuthConfig::Basic { username, password } => JmapAuth::Basic {
                username,
                password: password.get()?.into(),
            },
            JmapAuthConfig::Header { value } => JmapAuth::Header(value.get()?.into()),
        };

        Ok(JmapSession::new(self.server, self.tls.try_into()?, auth)?)
    }
}

#[cfg(feature = "jmap")]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum JmapAuthConfig {
    Bearer { token: Secret },
    Basic { username: String, password: Secret },
    Header { value: Secret },
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct TlsConfig {
    pub provider: Option<TlsProviderConfig>,
    #[serde(default)]
    pub rustls: RustlsConfig,
    pub cert: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum TlsProviderConfig {
    Rustls,
    NativeTls,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct RustlsConfig {
    pub crypto: Option<RustlsCryptoConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum RustlsCryptoConfig {
    Aws,
    Ring,
}

impl TryFrom<TlsConfig> for Tls {
    type Error = SecretError;

    fn try_from(config: TlsConfig) -> Result<Self, Self::Error> {
        Ok(Tls {
            provider: config.provider.map(|config| match config {
                TlsProviderConfig::Rustls => TlsProvider::Rustls,
                TlsProviderConfig::NativeTls => TlsProvider::NativeTls,
            }),
            rustls: Rustls {
                crypto: config.rustls.crypto.map(|config| match config {
                    RustlsCryptoConfig::Aws => RustlsCrypto::Aws,
                    RustlsCryptoConfig::Ring => RustlsCrypto::Ring,
                }),
            },
            cert: config.cert,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SaslConfig {
    pub mechanisms: Option<Vec<SaslMechanismConfig>>,
    pub login: Option<SaslLoginConfig>,
    pub plain: Option<SaslPlainConfig>,
    pub anonymous: Option<SaslAnonymousConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SaslMechanismConfig {
    Login,
    Plain,
    Anonymous,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SaslLoginConfig {
    #[serde(deserialize_with = "shell_expanded_string")]
    pub username: String,
    pub password: Secret,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SaslPlainConfig {
    pub authzid: Option<String>,
    #[serde(deserialize_with = "shell_expanded_string")]
    pub authcid: String,
    pub passwd: Secret,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SaslAnonymousConfig {
    pub message: Option<String>,
}

impl TryFrom<SaslConfig> for Sasl {
    type Error = SecretError;

    fn try_from(config: SaslConfig) -> Result<Self, Self::Error> {
        Ok(Sasl {
            mechanisms: match config.mechanisms {
                None => sasl_default_mechanisms(),
                Some(config) => config
                    .into_iter()
                    .map(|m| match m {
                        SaslMechanismConfig::Anonymous => SaslMechanism::Anonymous,
                        SaslMechanismConfig::Plain => SaslMechanism::Plain,
                        SaslMechanismConfig::Login => SaslMechanism::Login,
                    })
                    .collect(),
            },
            anonymous: match config.anonymous {
                None => None,
                Some(config) => Some(SaslAnonymous {
                    message: config.message,
                }),
            },
            plain: match config.plain {
                None => None,
                Some(config) => Some(SaslPlain {
                    authzid: config.authzid,
                    authcid: config.authcid,
                    passwd: config.passwd.get()?,
                }),
            },
            login: match config.login {
                None => None,
                Some(config) => Some(SaslLogin {
                    username: config.username,
                    password: config.password.get()?,
                }),
            },
        })
    }
}

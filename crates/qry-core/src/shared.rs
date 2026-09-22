use std::fmt::{Debug};
use async_trait::async_trait;
use crate::DatabaseType;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// When to use TLS. Same names and meaning as libpq's `sslmode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Disable,
    Allow,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    pub const ALL: [SslMode; 6] = [
        SslMode::Disable,
        SslMode::Allow,
        SslMode::Prefer,
        SslMode::Require,
        SslMode::VerifyCa,
        SslMode::VerifyFull,
    ];

    /// qry cannot open TLS connections yet. Modes that may fall back to plain
    /// text are accepted; modes that demand TLS are refused rather than
    /// silently connecting unencrypted.
    pub fn ensure_supported(self) -> Result<()> {
        match self {
            SslMode::Disable | SslMode::Allow | SslMode::Prefer => Ok(()),
            _ => bail!("SSL mode `{self}` needs TLS, which qry does not support yet"),
        }
    }
}

impl std::fmt::Display for SslMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SslMode::Disable => "disable",
            SslMode::Allow => "allow",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify-ca",
            SslMode::VerifyFull => "verify-full",
        })
    }
}

#[async_trait]
pub trait Database: Send + Sync{

    fn kind(&self) -> DatabaseType;
    fn label(&self) -> String;
    async fn query(&self, query: &str) -> Result<QueryResult>;
    async fn execute(&self, query: &str) -> Result<u64>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected: Option<u64>,
}
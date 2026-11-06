//! Process configuration. Secrets come from the environment only.

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub database_path: PathBuf,
    pub admin_github_ids: BTreeSet<u64>,
    pub github_client_id: Option<String>,
    pub github_client_secret: Option<String>,
    pub github_release_token: Option<String>,
    pub session_ttl: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let bind = std::env::var("NEXUS_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
        let database_path = std::env::var("NEXUS_DATABASE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("nexus-server.sqlite"));
        let admin_github_ids =
            parse_admin_ids(&std::env::var("NEXUS_ADMIN_GITHUB_IDS").unwrap_or_default())?;
        let github_client_id = empty_to_none(std::env::var("NEXUS_GITHUB_CLIENT_ID").ok());
        let github_client_secret = empty_to_none(std::env::var("NEXUS_GITHUB_CLIENT_SECRET").ok());
        let github_release_token = empty_to_none(std::env::var("NEXUS_GITHUB_RELEASE_TOKEN").ok());
        let hours: u64 = std::env::var("NEXUS_SESSION_TTL_HOURS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(24 * 30);
        if hours == 0 || hours > 24 * 90 {
            return Err("NEXUS_SESSION_TTL_HOURS must be between 1 and 2160".into());
        }
        Ok(Self {
            bind,
            database_path,
            admin_github_ids,
            github_client_id,
            github_client_secret,
            github_release_token,
            session_ttl: Duration::from_secs(hours * 3600),
        })
    }

    pub fn auth_configured(&self) -> bool {
        self.github_client_id.is_some() && self.github_client_secret.is_some()
    }

    pub fn is_admin(&self, github_id: u64) -> bool {
        self.admin_github_ids.contains(&github_id)
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Config")
            .field("bind", &self.bind)
            .field("database_path", &self.database_path)
            .field("admin_github_ids", &self.admin_github_ids)
            .field(
                "github_client_id",
                &self.github_client_id.as_ref().map(|_| "<set>"),
            )
            .field(
                "github_client_secret",
                &self.github_client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "github_release_token",
                &self.github_release_token.as_ref().map(|_| "<redacted>"),
            )
            .field("session_ttl_secs", &self.session_ttl.as_secs())
            .finish()
    }
}

pub fn parse_admin_ids(raw: &str) -> Result<BTreeSet<u64>, String> {
    let mut ids = BTreeSet::new();
    for part in raw.split(|c: char| c == ',' || c.is_whitespace()) {
        if part.is_empty() {
            continue;
        }
        let id: u64 = part
            .parse()
            .map_err(|_| format!("admin GitHub id '{part}' is not a positive integer"))?;
        if id == 0 {
            return Err("admin GitHub id 0 is not valid".into());
        }
        ids.insert(id);
    }
    Ok(ids)
}

fn empty_to_none(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

//! HTTP shapes shared by the desktop launcher and `nexus-server`.
//!
//! This crate has no network code and no secrets. Authorization rules live in
//! the server. The desktop must not treat a catalog response as permission to
//! fetch a private GitHub URL.

use serde::{Deserialize, Serialize};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub github_id: u64,
    pub login: String,
    pub admin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStart {
    pub poll_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub interval_seconds: u64,
    pub expires_in: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePollRequest {
    pub poll_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DevicePoll {
    Pending {
        interval_seconds: u64,
    },
    SlowDown {
        interval_seconds: u64,
    },
    Approved {
        session_token: String,
        account: Account,
    },
    Denied,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "public" => Some(Self::Public),
            "private" => Some(Self::Private),
            _ => None,
        }
    }
}

/// An app a caller is allowed to see. Private repos are omitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogApp {
    pub id: String,
    pub name: String,
    #[serde(default = "default_project_kind")]
    pub kind: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_stamp: Option<StatusStamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Set only when `visibility` is public. Private source names stay on the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminApp {
    pub id: String,
    pub name: String,
    #[serde(default = "default_project_kind")]
    pub kind: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub visibility: Visibility,
    pub published: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertApp {
    pub id: String,
    pub name: String,
    #[serde(default = "default_project_kind")]
    pub kind: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_status: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    pub visibility: Visibility,
    #[serde(default)]
    pub published: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishRequest {
    pub published: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantRequest {
    pub login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub github_id: u64,
    pub login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseMeta {
    pub tag: String,
    pub asset_name: String,
    pub size: u64,
    pub sha256: String,
    /// `portable`, `archive`, or `handoff`.
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryChange {
    pub name: String,
    pub replacement: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoPreviewRequest {
    pub repo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoPreview {
    pub repo: String,
    pub description: String,
    pub readme_excerpt: String,
    pub summary: String,
}

/// GitHub Linguist byte counts, ranked by usage with deterministic ties.
pub fn top_languages(counts: std::collections::BTreeMap<String, u64>) -> Vec<String> {
    let mut values: Vec<_> = counts.into_iter().filter(|(_, bytes)| *bytes > 0).collect();
    values.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    values.into_iter().take(3).map(|(name, _)| name).collect()
}

pub const PROJECT_STATUSES: &[(&str, &str)] = &[
    ("released", "Released"),
    ("in_development", "In development"),
    ("seeking_funding", "Seeking funding for prototype"),
];

pub fn project_status_label(status: &str) -> &str {
    PROJECT_STATUSES
        .iter()
        .find(|(key, _)| *key == status)
        .map(|(_, label)| *label)
        .unwrap_or("No status")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusStamp {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusStampChange {
    pub id: String,
    pub name: Option<String>,
    pub color: Option<String>,
}

pub fn default_status_stamps() -> Vec<StatusStamp> {
    PROJECT_STATUSES
        .iter()
        .zip(["#52d69a", "#f2bc56", "#5dc8ed"])
        .map(|((id, name), color)| StatusStamp {
            id: (*id).into(),
            name: (*name).into(),
            color: color.into(),
        })
        .collect()
}

pub fn default_project_kind() -> String {
    "app".into()
}

#[cfg(test)]
mod language_tests {
    #[test]
    fn ranks_languages_by_bytes_not_alphabetically() {
        let counts = [
            ("Python".into(), 900),
            ("Shell".into(), 20),
            ("Rust".into(), 500),
            ("HTML".into(), 1),
            ("Empty".into(), 0),
        ]
        .into();
        assert_eq!(super::top_languages(counts), ["Python", "Rust", "Shell"]);
    }
}

//! `catalog/apps.toml` is the extension point. Refresh re-reads the file.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// One desktop tool the launcher can show.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    /// GitHub `owner/name`. Absent means the card stays Coming soon.
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    /// Relative path of the executable inside an extracted archive.
    #[serde(default)]
    pub executable: Option<String>,
    /// Case-insensitive substrings. The first one that matches a compatible asset wins.
    #[serde(default)]
    pub asset_preference: Vec<String>,
    /// Empty means every platform.
    #[serde(default)]
    pub platforms: Vec<Platform>,
}

/// Parsed catalog. TOML tables are `[[app]]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Catalog {
    #[serde(default, rename = "app")]
    pub apps: Vec<AppEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Linux,
    Windows,
    Macos,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::Macos => "macos",
        }
    }
}

/// What the window should do with an entry before any network call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// Ask GitHub for `releases/latest`.
    Fetch,
    /// No `repo`, so there is nothing to install.
    ComingSoon,
    /// `platforms` does not include this operating system.
    NotOnPlatform,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("catalog is not valid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("{0}")]
    Invalid(String),
}

pub fn current_platform() -> Platform {
    match std::env::consts::OS {
        "windows" => Platform::Windows,
        "macos" => Platform::Macos,
        _ => Platform::Linux,
    }
}

pub fn current_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" | "amd64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        other => other,
    }
}

pub fn availability(app: &AppEntry, platform: Platform) -> Availability {
    if !app.platforms.is_empty() && !app.platforms.contains(&platform) {
        return Availability::NotOnPlatform;
    }
    if app.repo.is_none() {
        Availability::ComingSoon
    } else {
        Availability::Fetch
    }
}

pub fn load_catalog(path: &Path) -> Result<Catalog, CatalogError> {
    let text = fs::read_to_string(path).map_err(|source| CatalogError::Io {
        path: path.display().to_string(),
        source,
    })?;
    parse_catalog(&text)
}

pub fn parse_catalog(text: &str) -> Result<Catalog, CatalogError> {
    let catalog: Catalog = toml::from_str(text)?;
    let catalog = normalize(catalog);
    validate(&catalog)?;
    Ok(catalog)
}

pub fn is_github_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) => is_repo_part(owner) && is_repo_part(name),
        _ => false,
    }
}

fn normalize(mut catalog: Catalog) -> Catalog {
    for app in &mut catalog.apps {
        app.id = app.id.trim().to_string();
        app.name = app.name.trim().to_string();
        app.summary = app.summary.trim().to_string();
        app.repo = app.repo.as_ref().and_then(|repo| {
            let trimmed = repo.trim().trim_end_matches(".git");
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        if let Some(home) = &app.homepage {
            let trimmed = home.trim();
            app.homepage = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(category) = &app.category {
            let trimmed = category.trim();
            app.category = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(executable) = &app.executable {
            let trimmed = executable.trim();
            app.executable = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        app.asset_preference.retain(|item| !item.trim().is_empty());
        for item in &mut app.asset_preference {
            *item = item.trim().to_string();
        }
    }
    catalog
}

fn validate(catalog: &Catalog) -> Result<(), CatalogError> {
    if catalog.apps.is_empty() {
        return Err(CatalogError::Invalid(
            "catalog has no [[app]] entries".into(),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for app in &catalog.apps {
        if !is_slug(&app.id) {
            return Err(CatalogError::Invalid(format!(
                "app id '{}' must be a lowercase slug such as stat-tracker",
                app.id
            )));
        }
        if !seen.insert(app.id.clone()) {
            return Err(CatalogError::Invalid(format!(
                "duplicate app id '{}'",
                app.id
            )));
        }
        if app.name.is_empty() {
            return Err(CatalogError::Invalid(format!(
                "app '{}' is missing a name",
                app.id
            )));
        }
        if let Some(repo) = &app.repo {
            if !is_github_repo(repo) {
                return Err(CatalogError::Invalid(format!(
                    "app '{}' repo '{}' must be owner/name",
                    app.id, repo
                )));
            }
        }
        if let Some(executable) = &app.executable {
            if let Err(reason) = check_relative(executable) {
                return Err(CatalogError::Invalid(format!(
                    "app '{}' executable: {reason}",
                    app.id
                )));
            }
        }
    }
    Ok(())
}

fn is_slug(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes
        .iter()
        .any(|b| !b.is_ascii_lowercase() && !b.is_ascii_digit() && *b != b'-')
    {
        return false;
    }
    !value.starts_with('-') && !value.ends_with('-') && !value.contains("--")
}

fn is_repo_part(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !part.starts_with('.')
        && !part.ends_with('.')
}

fn check_relative(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Err("must be a relative path".into());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err("must not contain '..' or a drive prefix".into()),
        }
    }
    if path.as_os_str().is_empty() {
        return Err("must not be empty".into());
    }
    Ok(())
}

/// Catalog path for a normal launch.
///
/// An explicit path wins. Otherwise `catalog/apps.toml` in the working
/// directory, then the same file next to the executable (including
/// `target/release`).
pub fn resolve_catalog_path(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    let cwd = PathBuf::from("catalog").join("apps.toml");
    if cwd.exists() {
        return cwd;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for rel in [
                Path::new("catalog").join("apps.toml"),
                Path::new("..").join("catalog").join("apps.toml"),
                Path::new("..").join("..").join("catalog").join("apps.toml"),
            ] {
                let candidate = dir.join(rel);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    cwd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_catalog_lists_the_four_tools() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("catalog/apps.toml");
        let catalog = load_catalog(&path).unwrap();
        let ids: Vec<_> = catalog.apps.iter().map(|app| app.id.as_str()).collect();
        assert_eq!(
            ids,
            ["stat-tracker", "snipscratch", "snaplapse", "frameweaver"]
        );
        assert!(catalog.apps.iter().all(|app| app.repo.is_some()));
        assert!(catalog.apps.iter().all(|app| !app.summary.is_empty()));
    }

    #[test]
    fn missing_repo_is_coming_soon() {
        let catalog = parse_catalog(
            r#"
            [[app]]
            id = "draft"
            name = "Draft"
            summary = "Not published."
            "#,
        )
        .unwrap();
        assert_eq!(
            availability(&catalog.apps[0], Platform::Linux),
            Availability::ComingSoon
        );
    }

    #[test]
    fn platform_filter_skips_fetch() {
        let catalog = parse_catalog(
            r#"
            [[app]]
            id = "win-only"
            name = "Win Only"
            repo = "Straconis/win-only"
            platforms = ["windows"]
            "#,
        )
        .unwrap();
        assert_eq!(
            availability(&catalog.apps[0], Platform::Linux),
            Availability::NotOnPlatform
        );
        assert_eq!(
            availability(&catalog.apps[0], Platform::Windows),
            Availability::Fetch
        );
    }

    #[test]
    fn rejects_duplicate_ids_and_bad_repos() {
        let dup = parse_catalog(
            r#"
            [[app]]
            id = "same"
            name = "One"
            [[app]]
            id = "same"
            name = "Two"
            "#,
        );
        assert!(dup.unwrap_err().to_string().contains("duplicate"));

        let repo = parse_catalog(
            r#"
            [[app]]
            id = "tool"
            name = "Tool"
            repo = "https://github.com/Straconis/tool"
            "#,
        );
        assert!(repo.unwrap_err().to_string().contains("owner/name"));
    }

    #[test]
    fn strips_git_suffix_and_blank_repo() {
        let catalog = parse_catalog(
            r#"
            [[app]]
            id = "tool"
            name = "Tool"
            repo = "Straconis/tool.git"
            [[app]]
            id = "soon"
            name = "Soon"
            repo = "  "
            "#,
        )
        .unwrap();
        assert_eq!(catalog.apps[0].repo.as_deref(), Some("Straconis/tool"));
        assert_eq!(catalog.apps[1].repo, None);
    }

    #[test]
    fn rejects_parent_executable() {
        let err = parse_catalog(
            r#"
            [[app]]
            id = "tool"
            name = "Tool"
            executable = "../bin/tool"
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("executable"));
    }
}

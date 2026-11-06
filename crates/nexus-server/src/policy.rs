//! Field checks for catalog rows. These run before anything is stored or fetched.

use nexus_protocol::Visibility;

pub fn parse_app_id(id: &str) -> Result<String, &'static str> {
    if id.len() > 64 || id.is_empty() {
        return Err("app id must be 1 to 64 characters");
    }
    let bytes = id.as_bytes();
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return Err("app id must be a lowercase slug");
    }
    if bytes
        .iter()
        .any(|byte| !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && *byte != b'-')
        || id.starts_with('-')
        || id.ends_with('-')
        || id.contains("--")
    {
        return Err("app id must be a lowercase slug");
    }
    Ok(id.to_string())
}

pub fn parse_repo(repo: &str) -> Result<String, &'static str> {
    let repo = repo.trim();
    let Some((owner, name)) = repo.split_once('/') else {
        return Err("repo must be owner/name");
    };
    if owner.is_empty()
        || name.is_empty()
        || repo.split('/').count() != 2
        || !part_ok(owner)
        || !part_ok(name)
    {
        return Err("repo must be a single owner/name");
    }
    Ok(format!("{owner}/{name}"))
}

fn part_ok(part: &str) -> bool {
    (1..=100).contains(&part.len())
        && !part.starts_with('.')
        && !part.ends_with('.')
        && !part.contains("..")
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

pub fn parse_login(login: &str) -> Result<String, &'static str> {
    let login = login.trim();
    if !(1..=39).contains(&login.len())
        || login.starts_with('-')
        || login.ends_with('-')
        || login.contains("--")
        || !login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("GitHub login is not a single username");
    }
    Ok(login.to_string())
}

pub fn parse_homepage(url: &str) -> Result<String, &'static str> {
    let url = url.trim();
    let parsed = reqwest::Url::parse(url).map_err(|_| "homepage must be an https URL")?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() || !parsed.username().is_empty() {
        return Err("homepage must be https and must not embed credentials");
    }
    Ok(parsed.to_string())
}

pub fn clean_optional(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub fn visibility_label(visibility: Visibility) -> &'static str {
    visibility.as_str()
}

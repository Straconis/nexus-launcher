//! Normalize pasted GitHub links without confusing repository names with local IDs.
#[derive(Debug, PartialEq, Eq)]
pub struct RepoFields {
    pub repo: String,
    pub id: String,
    pub name: String,
    pub homepage: String,
}

pub fn parse(input: &str) -> Result<RepoFields, String> {
    let input = input.trim();
    let path = if input.starts_with("https://") || input.starts_with("http://") {
        let url = reqwest::Url::parse(input).map_err(|_| "Invalid GitHub URL")?;
        if !matches!(url.host_str(), Some("github.com" | "www.github.com"))
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Paste a github.com repository URL or owner/repository.".into());
        }
        url.path()
            .trim_matches('/')
            .split('/')
            .take(2)
            .collect::<Vec<_>>()
            .join("/")
    } else {
        input.trim_matches('/').to_string()
    };
    let repo = path.strip_suffix(".git").unwrap_or(&path);
    if !crate::catalog::is_github_repo(repo) {
        return Err("Repository must be owner/repository.".into());
    }
    let name = repo.split('/').nth(1).unwrap_or_default();
    let id = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|v| !v.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-");
    if id.is_empty() || id.len() > 64 {
        return Err("Repository name needs a custom app ID (1–64 characters).".into());
    }
    let display = name
        .split(['-', '_', '.'])
        .filter(|v| !v.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    Ok(RepoFields {
        repo: repo.into(),
        id,
        name: display,
        homepage: format!("https://github.com/{repo}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_repo_links_without_changing_the_real_repo_slug() {
        let fields =
            parse("https://github.com/Straconis/stat_tracker.git/?tab=readme#intro").unwrap();
        assert_eq!(fields.repo, "Straconis/stat_tracker");
        assert_eq!(fields.id, "stat-tracker");
        assert_eq!(fields.name, "Stat Tracker");
        assert_eq!(
            parse("https://github.com/Straconis/stat_tracker/tree/main/src")
                .unwrap()
                .repo,
            fields.repo
        );
        assert!(parse("https://github.com.evil.test/Straconis/tool").is_err());
        assert!(parse("https://secret@github.com/Straconis/tool").is_err());
    }
}
